//! بروتوكول ZKTeco (منفذ 4370) بلغة Rust نقي — مطابق لمنطق pyzk المُختبَر.
//! لا يحتاج أي DLL؛ يعمل على ويندوز ولينكس.
use super::{DeviceBackend, DeviceInfo, RawPunch, RawUser};
use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use std::str;

const TOP1: u16 = 0x5050;
const TOP2: u16 = 0x7282;
const U: u32 = 65535;

// أوامر
const CMD_CONNECT: u16 = 1000;
const CMD_AUTH: u16 = 1102;
const CMD_EXIT: u16 = 1001;
const CMD_GET_VERSION: u16 = 1100;
const CMD_OPTIONS_RRQ: u16 = 11;
const CMD_GET_FREE_SIZES: u16 = 50;
const CMD_GET_TIME: u16 = 201;
const CMD_SET_TIME: u16 = 202;
const CMD_USER_WRQ: u16 = 8;
const CMD_USERTEMP_RRQ: u16 = 9;
const CMD_ATTLOG_RRQ: u16 = 13;
const _CMD_PREPARE_BUFFER: u16 = 1503;
const _CMD_READ_BUFFER: u16 = 1504;
const CMD_ACK_OK: u16 = 2000;
const CMD_ACK_UNAUTH: u16 = 2005;
const CMD_PREPARE_DATA: u16 = 1500;
const CMD_DATA: u16 = 1501;
const FCT_USER: i32 = 5;

fn checksum(bytes: &[u8]) -> u16 {
    let mut cs: u32 = 0;
    let mut p = bytes;
    while p.len() > 1 {
        cs += (p[0] as u32) | ((p[1] as u32) << 8);
        p = &p[2..];
        if cs > U {
            cs -= U;
        }
    }
    if !p.is_empty() {
        cs += p[0] as u32;
    }
    while cs > U {
        cs -= U;
    }
    let cs = !(cs as i32);
    (cs & 0xFFFF) as u16
}

/// توليد مفتاح الاتصال المشوّر (MakeKey) — مطابق لـ commpro.c.
fn make_commkey(key: u32, session_id: u16, ticks: u8) -> [u8; 4] {
    let mut k: u32 = 0;
    for i in 0..32 {
        k = if (key >> i) & 1 == 1 { (k << 1) | 1 } else { k << 1 };
    }
    k = k.wrapping_add(session_id as u32);
    let b = k.to_le_bytes();
    let mut x = [b[0] ^ 0x5A, b[1] ^ 0x4B, b[2] ^ 0x53, b[3] ^ 0x4F];
    // swap 16-bit halves
    x.swap(0, 2);
    x.swap(1, 3);
    let t = ticks;
    [x[0] ^ t, x[1] ^ t, t, x[3] ^ t]
}

/// تحويل بايتات مُنهيّة بـ NUL إلى نص (يتجاهل الحشو).
fn bytes_to_str(b: &[u8]) -> String {
    let end = b.iter().position(|&x| x == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).to_string()
}

/// فك ترميز وقت ZKTeco (u32) إلى ISO8601.
fn decode_time(v: u32) -> String {
    let mut t = v as u64;
    let sec = t % 60; t /= 60;
    let min = t % 60; t /= 60;
    let hour = t % 24; t /= 24;
    let day = t % 31 + 1; t /= 31;
    let month = t % 12 + 1; t /= 12;
    let year = t + 2000;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        year, month, day, hour, min, sec
    )
}

/// ترميز وقت إلى صيغة ZKTeco (للاستخدام في set_time).
fn encode_time(iso: &str) -> Result<u32> {
    use chrono::{Datelike, Timelike};
    let dt = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S")
        .unwrap_or_else(|_| chrono::Utc::now().naive_utc());
    let v = (((dt.year() as u64 % 100) * 12 * 31
        + ((dt.month() as u64 - 1) * 31)
        + dt.day() as u64
        - 1) * 24 * 60 * 60
        + dt.hour() as u64 * 60 * 60
        + dt.minute() as u64 * 60
        + dt.second() as u64) as u32;
    Ok(v)
}

pub struct ZkProtocol {
    sock: Option<TcpStream>,
    ip: String,
    port: u16,
    comm_key: u32,
    session_id: u16,
    reply_id: u16,
}

impl ZkProtocol {
    pub fn new(ip: &str, port: u16, comm_key: u32) -> Self {
        Self {
            sock: None,
            ip: ip.to_string(),
            port,
            comm_key,
            session_id: 0,
            reply_id: (U - 1) as u16,
        }
    }

    fn build_frame(&mut self, cmd: u16, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.extend_from_slice(&cmd.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // checksum placeholder
        buf.extend_from_slice(&self.session_id.to_le_bytes());
        buf.extend_from_slice(&self.reply_id.to_le_bytes());
        buf.extend_from_slice(payload);
        let cs = checksum(&buf);
        self.reply_id = self.reply_id.wrapping_add(1);
        // أعد البناء مع checksum صحيح و reply_id محدّث
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&cmd.to_le_bytes());
        out.extend_from_slice(&cs.to_le_bytes());
        out.extend_from_slice(&self.session_id.to_le_bytes());
        out.extend_from_slice(&self.reply_id.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn send_frame(&mut self, cmd: u16, payload: &[u8]) -> Result<()> {
        let pkt = self.build_frame(cmd, payload);
        let mut top = Vec::with_capacity(8);
        top.extend_from_slice(&TOP1.to_le_bytes());
        top.extend_from_slice(&TOP2.to_le_bytes());
        top.extend_from_slice(&(pkt.len() as u32).to_le_bytes());
        top.extend_from_slice(&pkt);
        let sock = self.sock.as_mut().context("not connected")?;
        sock.write_all(&top).context("send failed")?;
        Ok(())
    }

    fn recv_exact(&mut self, n: usize) -> Result<Vec<u8>> {
        let sock = self.sock.as_mut().context("not connected")?;
        let mut buf = vec![0u8; n];
        sock.read_exact(&mut buf).context("recv failed")?;
        Ok(buf)
    }

    /// يقرأ إطار TCP واحداً ويعيد (cmd, payload) مع تحديث session/reply.
    fn read_frame(&mut self) -> Result<(u16, Vec<u8>)> {
        let head = self.recv_exact(8)?;
        let m1 = u16::from_le_bytes([head[0], head[1]]);
        let m2 = u16::from_le_bytes([head[2], head[3]]);
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if m1 != TOP1 || m2 != TOP2 {
            return Err(anyhow!("bad frame magic {m1:04x}{m2:04x}"));
        }
        let body = self.recv_exact(len)?;
        let cmd = u16::from_le_bytes([body[0], body[1]]);
        self.session_id = u16::from_le_bytes([body[4], body[5]]);
        self.reply_id = u16::from_le_bytes([body[6], body[7]]);
        Ok((cmd, body[8..].to_vec()))
    }

    /// يرسل أمراً ويعيد أول رد (cmd, payload).
    fn command(&mut self, cmd: u16, payload: &[u8]) -> Result<(u16, Vec<u8>)> {
        self.send_frame(cmd, payload)?;
        self.read_frame()
    }

    /// قراءة دفعة كبيرة عبر PREPARE_BUFFER / READ_BUFFER (مع تجميع الإطارات).
    fn read_with_buffer(&mut self, command: u16, fct: i32) -> Result<Vec<u8>> {
        let mut cs = Vec::new();
        cs.push(1u8); // type
        cs.extend_from_slice(&command.to_le_bytes());
        cs.extend_from_slice(&fct.to_le_bytes());
        cs.extend_from_slice(&0i32.to_le_bytes()); // ext
        let (cmd, payload) = self.command(_CMD_PREPARE_BUFFER, &cs)?;
        if cmd == CMD_DATA {
            return Ok(payload);
        }
        if cmd != CMD_ACK_OK || payload.len() < 5 {
            return Err(anyhow!("PREPARE_BUFFER rejected cmd={cmd}"));
        }
        let total = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]) as usize;
        const CHUNK: usize = 0x4000;
        let mut collected = Vec::with_capacity(total);
        let mut start: i32 = 0;
        while collected.len() < total {
            let want = std::cmp::min(CHUNK, total - collected.len()) as i32;
            let mut rc = Vec::new();
            rc.extend_from_slice(&start.to_le_bytes());
            rc.extend_from_slice(&want.to_le_bytes());
            let (cmd, payload) = self.command(_CMD_READ_BUFFER, &rc)?;
            if cmd == CMD_PREPARE_DATA {
                let ann =
                    u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                let mut got = Vec::with_capacity(ann);
                while got.len() < ann {
                    let (c2, p2) = self.read_frame()?;
                    if c2 == CMD_DATA {
                        got.extend_from_slice(&p2);
                    } else if c2 == CMD_ACK_OK {
                        break;
                    }
                }
                start += got.len() as i32;
                collected.extend_from_slice(&got);
            } else if cmd == CMD_DATA {
                start += payload.len() as i32;
                collected.extend_from_slice(&payload);
            } else if cmd == CMD_ACK_OK {
                break;
            } else {
                return Err(anyhow!("READ_BUFFER unexpected cmd={cmd}"));
            }
        }
        Ok(collected)
    }

    fn options(&mut self, key: &str) -> Result<String> {
        let mut p = key.as_bytes().to_vec();
        p.push(0);
        let (cmd, payload) = self.command(CMD_OPTIONS_RRQ, &p)?;
        if cmd != CMD_ACK_OK {
            return Err(anyhow!("options {key} rejected"));
        }
        let s = String::from_utf8_lossy(&payload).to_string();
        Ok(s.split('=').last().unwrap_or("").trim_matches('\0').to_string())
    }

    fn read_sizes(&mut self) -> Result<(i64, i64, i64)> {
        let (cmd, payload) = self.command(CMD_GET_FREE_SIZES, &[])?;
        if cmd != CMD_ACK_OK || payload.len() < 80 {
            return Err(anyhow!("sizes rejected"));
        }
        let g = |i: usize| -> i64 {
            let o = i * 4;
            i32::from_le_bytes([payload[o], payload[o + 1], payload[o + 2], payload[o + 3]]) as i64
        };
        Ok((g(4), g(6), g(8))) // users, fingers, records
    }
}

impl DeviceBackend for ZkProtocol {
    fn connect(&mut self) -> Result<DeviceInfo> {
        let stream = TcpStream::connect((self.ip.as_str(), self.port))
            .with_context(|| format!("connect {}:{} failed", self.ip, self.port))?;
        stream.set_read_timeout(Some(Duration::from_secs(15)))?;
        stream.set_write_timeout(Some(Duration::from_secs(15)))?;
        self.sock = Some(stream);
        self.session_id = 0;
        self.reply_id = (U - 1) as u16;
        let (cmd, _) = self.command(CMD_CONNECT, &[])?;
        if cmd == CMD_ACK_UNAUTH {
            let key = make_commkey(self.comm_key, self.session_id, 50);
            let (cmd2, _) = self.command(CMD_AUTH, &key)?;
            if cmd2 != CMD_ACK_OK {
                return Err(anyhow!("auth rejected (comm key?)"));
            }
        } else if cmd != CMD_ACK_OK {
            return Err(anyhow!("connect rejected cmd={cmd}"));
        }
        self.info()
    }

    fn info(&mut self) -> Result<DeviceInfo> {
        let firmware = self
            .command(CMD_GET_VERSION, &[])
            .map(|(_, p)| String::from_utf8_lossy(&p).trim_matches('\0').to_string())
            .unwrap_or_default();
        let platform = self.options("~Platform").unwrap_or_default();
        let serial = self.options("~SerialNumber").unwrap_or_default();
        let device_name = self.options("~DeviceName").unwrap_or_default();
        let (users, fingers, records) = self.read_sizes().unwrap_or((0, 0, 0));
        Ok(DeviceInfo {
            connected: true,
            firmware,
            platform,
            serial,
            device_name,
            users,
            fingerprints: fingers,
            records,
        })
    }

    fn pull_users(&mut self) -> Result<Vec<RawUser>> {
        let data = self.read_with_buffer(CMD_USERTEMP_RRQ, FCT_USER)?;
        if data.len() < 4 {
            return Ok(vec![]);
        }
        let total = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let (users, _, _) = self.read_sizes().unwrap_or((0, 0, 0));
        let psize = if users > 0 { total / users as usize } else { 72 };
        let body = &data[4..];
        let mut out = Vec::new();
        if psize == 72 {
            for c in body.chunks_exact(72) {
                let uid = u16::from_le_bytes([c[0], c[1]]) as i64;
                let privilege = c[2] as i64;
                let name = bytes_to_str(&c[13..37]);
                let card = u32::from_le_bytes([c[37], c[38], c[39], c[40]]);
                let user_id = bytes_to_str(&c[53..77]);
                let pin = if user_id.is_empty() { uid.to_string() } else { user_id };
                out.push((pin, name, card, privilege));
            }
        } else if psize == 28 {
            for c in body.chunks_exact(28) {
                let uid = u16::from_le_bytes([c[0], c[1]]) as i64;
                let privilege = c[2] as i64;
                let name = bytes_to_str(&c[8..16]);
                let user_id = &c[24..28];
                let uid_num = u32::from_le_bytes([user_id[0], user_id[1], user_id[2], user_id[3]]);
                out.push((uid.to_string(), name, 0, privilege));
                let _ = uid_num;
            }
        }
        Ok(out)
    }

    fn pull_attendance(&mut self) -> Result<Vec<RawPunch>> {
        let data = self.read_with_buffer(CMD_ATTLOG_RRQ, 0)?;
        if data.len() < 4 {
            return Ok(vec![]);
        }
        let total = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let (_, _, records) = self.read_sizes().unwrap_or((0, 0, 0));
        let recsize = if records > 0 { total / records as usize } else { 16 };
        let body = &data[4..];

        // بناء خريطة uid → pin من الموظفين (لترجمة السجلات التي تحمل uid رقمياً)
        let users = self.pull_users().unwrap_or_default();
        let uid_to_pin: std::collections::HashMap<String, String> = users
            .iter()
            .map(|(pin, _, _, _)| (pin.clone(), pin.clone()))
            .collect();

        let mut out = Vec::new();
        let mut push = |pin: String, ts_v: u32, status: u8, punch: u8, wc: u32| {
            out.push((pin, decode_time(ts_v), punch as i64, wc as i64, status as i64));
        };
        match recsize {
            8 => {
                for c in body.chunks_exact(8) {
                    let uid = u16::from_le_bytes([c[0], c[1]]);
                    let status = c[2];
                    let ts = u32::from_le_bytes([c[3], c[4], c[5], c[6]]);
                    let punch = c[7];
                    let pin = uid_to_pin
                        .get(&uid.to_string())
                        .cloned()
                        .unwrap_or_else(|| uid.to_string());
                    push(pin, ts, status, punch, 0);
                }
            }
            16 => {
                for c in body.chunks_exact(16) {
                    let uid = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    let ts = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                    let status = c[8];
                    let punch = c[9];
                    let wc = u32::from_le_bytes([c[12], c[13], c[14], c[15]]);
                    let pin = uid_to_pin
                        .get(&uid.to_string())
                        .cloned()
                        .unwrap_or_else(|| uid.to_string());
                    push(pin, ts, status, punch, wc);
                }
            }
            // صيغة موسّعة (40 بايت كما رصدنا على K50 Pro):
            // uid(u16)@0, pin str@2..26, ts(u32)@28, status@32, punch@33
            sz if sz >= 32 => {
                for c in body.chunks_exact(sz) {
                    let uid = u16::from_le_bytes([c[0], c[1]]);
                    let pin_str = bytes_to_str(&c[2..26]);
                    let ts = u32::from_le_bytes([c[28], c[29], c[30], c[31]]);
                    let status = *c.get(32).unwrap_or(&0);
                    let punch = *c.get(33).unwrap_or(&0);
                    let pin = if !pin_str.is_empty() {
                        pin_str
                    } else {
                        uid_to_pin
                            .get(&uid.to_string())
                            .cloned()
                            .unwrap_or_else(|| uid.to_string())
                    };
                    push(pin, ts, status, punch, 0);
                }
            }
            _ => return Err(anyhow!("unsupported record size {recsize}")),
        }
        Ok(out)
    }

    fn get_time_iso(&mut self) -> Result<String> {
        let (cmd, payload) = self.command(CMD_GET_TIME, &[])?;
        if cmd != CMD_ACK_OK || payload.len() < 4 {
            return Err(anyhow!("get_time rejected"));
        }
        let v = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        Ok(decode_time(v))
    }

    fn set_time(&mut self, iso: &str) -> Result<()> {
        let v = encode_time(iso)?;
        let (cmd, _) = self.command(CMD_SET_TIME, &v.to_le_bytes())?;
        if cmd != CMD_ACK_OK {
            return Err(anyhow!("set_time rejected"));
        }
        Ok(())
    }

    fn add_user(&mut self, pin: &str, name: &str) -> Result<()> {
        // حزمة المستخدم 72 بايت (zk8)
        let mut c = vec![0u8; 72];
        let uid: u16 = pin.parse().unwrap_or(0);
        c[0..2].copy_from_slice(&uid.to_le_bytes());
        c[2] = 0; // privilege = user
        let nb = name.as_bytes();
        c[13..13 + nb.len().min(24)].copy_from_slice(&nb[..nb.len().min(24)]);
        let pb = pin.as_bytes();
        c[53..53 + pb.len().min(24)].copy_from_slice(&pb[..pb.len().min(24)]);
        let (cmd, _) = self.command(CMD_USER_WRQ, &c)?;
        if cmd != CMD_ACK_OK {
            return Err(anyhow!("add_user rejected"));
        }
        // refresh data
        let _ = self.command(1013, &[]);
        Ok(())
    }

    fn close(&mut self) {
        let _ = self.command(CMD_EXIT, &[]);
        if let Some(s) = self.sock.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}
