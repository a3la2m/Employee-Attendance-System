//! بروتوكول ZKTeco (منفذ 4370) بلغة Rust نقي — مطابق لمنطق pyzk المُختبَر.
//! لا يحتاج أي DLL؛ يعمل على ويندوز ولينكس.
use super::{DeviceBackend, DeviceInfo, RawPunch, RawUser};
use anyhow::{anyhow, Context, Result};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use std::str;

const TOP1: u16 = 0x5050;
// 0x7D82 (32130) — سحر الإطار المُثبت تجريبياً على K50 Pro. القيمة 0x7282
// في بعض نسخ pyzk تؤدي إلى رفض فوري (RST) من الجهاز = "recv failed".
const TOP2: u16 = 0x7D82;
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

    /// يبني إطاراً كاملاً (بدون رأس TCP). checksum يُحسب على **نفس البكادات
    /// التي ستُرسل** (حقل checksum = 0) — هذا ما يتحقّق منه الجهاز حيّاً
    /// (صيغة complement الـ16bit: `!S & 0xFFFF`).
    fn build_frame(&mut self, cmd: u16, payload: &[u8]) -> Vec<u8> {
        // reply_id يزيد أولاً بطريقة pyzk: % 65535 (وليس wrapping % 65536)
        self.reply_id = ((self.reply_id as u32 + 1) % U) as u16;
        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.extend_from_slice(&cmd.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // placeholder — يُصفّى عند التحقق
        buf.extend_from_slice(&self.session_id.to_le_bytes());
        buf.extend_from_slice(&self.reply_id.to_le_bytes());
        buf.extend_from_slice(payload);
        let cs = checksum(&buf);
        buf[2..4].copy_from_slice(&cs.to_le_bytes());
        buf
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
        let mut start: usize = 0;
        // نقرأ دفعةً دفعة حتى نجمع total. الـ ACK_OK الختامي لكل دفعة يُستهلك
        // ضمن الدفعة نفسها ثم يُطلب CHUNK التالي — مطابق لـ pyzk read_with_buffer.
        while collected.len() < total {
            let want = std::cmp::min(CHUNK, total - start);
            let mut rc = Vec::new();
            rc.extend_from_slice(&(start as i32).to_le_bytes());
            rc.extend_from_slice(&(want as i32).to_le_bytes());
            let (cmd, payload) = self.command(_CMD_READ_BUFFER, &rc)?;
            if cmd == CMD_PREPARE_DATA {
                let ann =
                    u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                let mut got = Vec::with_capacity(ann);
                loop {
                    let (c2, p2) = self.read_frame()?;
                    if c2 == CMD_DATA {
                        got.extend_from_slice(&p2);
                    } else if c2 == CMD_ACK_OK {
                        break; // نهاية هذه الدفعة
                    } else {
                        return Err(anyhow!("chunk data unexpected cmd={c2}"));
                    }
                }
                if got.is_empty() {
                    break; // لا مزيد من البيانات عند الجهاز
                }
                start += got.len();
                collected.extend_from_slice(&got);
            } else if cmd == CMD_DATA {
                start += payload.len();
                collected.extend_from_slice(&payload);
            } else if cmd == CMD_ACK_OK {
                break; // الجهاز أنهى البث قبل total
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

    /// كل سجلات المستخدمين مع الـ uid الخام — يُستعمل أيضاً لإعادة استخدام
    /// نفس الـ uid عند تحديث اسم مستخدم موجود (وإلا خلق الجهاز سجلاً مكرراً).
    fn pull_user_records(&mut self) -> Result<Vec<(u16, String, String, u32, i64, [u8; 8])>> {
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
            // تخطيط zk8 الرسمي (pyzk: unpack '<HB8s24sIx7sx24s'):
            // uid(u16)@0، privilege@2، password@3..11، name@11..35،
            // card(u32)@35..39، group@40..47، pin/user_id@48..72
            for c in body.chunks_exact(72) {
                let uid = u16::from_le_bytes([c[0], c[1]]);
                let privilege = c[2] as i64;
                let password: [u8; 8] = c[3..11].try_into().unwrap();
                let name = bytes_to_str(&c[11..35]);
                let card = u32::from_le_bytes([c[35], c[36], c[37], c[38]]);
                let user_id = bytes_to_str(&c[48..72]);
                let pin = if user_id.is_empty() { uid.to_string() } else { user_id };
                out.push((uid, pin, name, card, privilege, password));
            }
        } else if psize == 28 {
            // zk6: '<HB5s8sIxBhI' — pin = user_id رقمي@24، password@3..8 (5 بايت)
            for c in body.chunks_exact(28) {
                let uid = u16::from_le_bytes([c[0], c[1]]);
                let privilege = c[2] as i64;
                let mut password = [0u8; 8];
                password[..5].copy_from_slice(&c[3..8]);
                let name = bytes_to_str(&c[8..16]);
                let uid_num = u32::from_le_bytes([c[24], c[25], c[26], c[27]]);
                let pin = if uid_num != 0 {
                    uid_num.to_string()
                } else {
                    uid.to_string()
                };
                out.push((uid, pin, name, 0, privilege, password));
            }
        }
        Ok(out)
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
        Ok(self
            .pull_user_records()?
            .into_iter()
            .map(|(_, pin, name, card, privilege, _)| (pin, name, card, privilege))
            .collect())
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

    /// إرسال/تحديث مستخدم على الجهاز: نبحث عن uid المحفوظ لهذا الـpin
    /// (وإلا لخلق الجهاز سجلاً مكرراً) ونحافظ على امتيازه وبطاقته.
    fn add_user(&mut self, pin: &str, name: &str) -> Result<()> {
        let existing = self.pull_user_records().unwrap_or_default();
        let found = existing.iter().find(|(_, p, ..)| p == pin);
        let (uid, privilege, card, password) = match found {
            Some((u, _, _, card, pv, pw)) => (*u, (*pv).min(255) as u8, *card, *pw),
            None => (pin.parse().unwrap_or(0), 0u8, 0u32, [0u8; 8]),
        };
        // حزمة 72 بايت بالترتيب الرسمي pyzk '<HB8s24sIx7sx24s'
        let mut c = vec![0u8; 72];
        c[0..2].copy_from_slice(&uid.to_le_bytes());
        c[2] = privilege;
        c[3..11].copy_from_slice(&password);
        let nb = name.as_bytes();
        let mut nlen = nb.len().min(24);
        // لا نقطع حرف UTF-8 في منتصفه: نتراجع لآخر حدّ حرف سليم
        while nlen > 0 && nlen < nb.len() && (nb[nlen] & 0xC0) == 0x80 {
            nlen -= 1;
        }
        c[11..11 + nlen].copy_from_slice(&nb[..nlen]);
        c[35..39].copy_from_slice(&card.to_le_bytes());
        let pb = pin.as_bytes();
        let mut plen = pb.len().min(24);
        while plen > 0 && plen < pb.len() && (pb[plen] & 0xC0) == 0x80 {
            plen -= 1;
        }
        c[48..48 + plen].copy_from_slice(&pb[..plen]);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// checksum الإطار يجب أن يتطابق مع إعادة الحساب بعد تصفير الحقل —
    /// بالضبط التحقق الذي يجريه الجهاز (مُثبت تجريبياً 3/3 على K50 Pro).
    #[test]
    fn frame_checksum_matches_device_verification() {
        let mut p = ZkProtocol::new("0.0.0.0", 4370, 0);
        for (i, cmd) in [1000u16, 11, 50, 1503, 1504].into_iter().enumerate() {
            let frame = p.build_frame(cmd, &[1, 2, 3, 4]);
            assert_eq!(u16::from_le_bytes([frame[0], frame[1]]), cmd);
            // reply_id متسلسل: الأول = 0 ثم يزيد (طريقة pyzk % 65535)
            assert_eq!(u16::from_le_bytes([frame[6], frame[7]]), i as u16);
            let mut zeroed = frame.clone();
            zeroed[2] = 0;
            zeroed[3] = 0;
            let cs = checksum(&zeroed);
            assert_eq!(
                u16::from_le_bytes([frame[2], frame[3]]),
                cs,
                "checksum must equal the mod-65536 complement of the exact sent bytes"
            );
        }
        assert_eq!(p.reply_id, 4); // (65534 + 5) % 65535
    }

    /// سحر الإطار المُثبت على K50 Pro؛ القيمة القديمة 0x7282 تُسبب RST فوري.
    #[test]
    fn frame_magic_is_7d82() {
        assert_eq!(TOP1, 0x5050);
        assert_eq!(TOP2, 0x7D82);
    }

    /// اختبار حيّ ضد الجهاز الحقيقي — يُشغَّل يدوياً فقط (يتجاهله CI):
    /// ZK_IP=192.168.1.205 cargo test live_device -- --ignored --nocapture
    #[test]
    #[ignore = "يحتاج جهاز بصمة على الشبكة"]
    fn live_device_smoke() {
        let ip = std::env::var("ZK_IP").unwrap_or_else(|_| "192.168.1.205".to_string());
        let mut d = ZkProtocol::new(&ip, 4370, 0);
        let info = d.connect().expect("connect failed");
        println!(
            "connected: {} {} serial={} users={} records={}",
            info.device_name, info.firmware, info.serial, info.users, info.records
        );
        assert!(info.connected);
        let users = d.pull_users().expect("pull_users failed");
        println!("users pulled: {}  first={:?}", users.len(), users.first());
        assert_eq!(users.len() as i64, info.users, "user count mismatch");
        let att = d.pull_attendance().expect("pull_attendance failed");
        println!(
            "attendance pulled: {}\n  first={:?}\n  last={:?}",
            att.len(),
            att.first(),
            att.last()
        );
        assert_eq!(att.len() as i64, info.records, "record count mismatch");
        let now_iso = d.get_time_iso().expect("get_time failed");
        println!("device clock: {now_iso}");
        d.close();
    }

    /// اختبار حيّ لزر «إرسال الاسم إلى الجهاز»: يرسل اسماً عربياً للمستخدم
    /// pin=9999 ثم يتحقق من (عدم خلق مستخدم مكرر + uid + الامتياز + البطاقة
    /// + كلمة السر)، ثم يُرجع الاسم الأصلي كما كان. يتجاهله CI:
    /// ZK_IP=192.168.1.205 cargo test live_push -- --ignored --nocapture
    #[test]
    #[ignore = "يحتاج جهاز بصمة على الشبكة"]
    fn live_push_user_smoke() {
        let ip = std::env::var("ZK_IP").unwrap_or_else(|_| "192.168.1.205".to_string());
        let mut zk = ZkProtocol::new(&ip, 4370, 0);
        zk.connect().expect("connect failed");

        let before = zk.pull_user_records().expect("pull before");
        println!("قبل: {} مستخدمين", before.len());
        let t0 = before
            .iter()
            .find(|(_, p, ..)| p == "9999")
            .expect("المستخدم pin=9999 غير موجود");
        let (uid0, card0, priv0, pw0, name0) = (t0.0, t0.3, t0.4, t0.5, t0.2.clone());
        println!("  uid={uid0} priv={priv0} card={card0} name={name0:?} pw={pw0:?}");

        zk.add_user("9999", "اختبار عربي").expect("add_user failed");

        let mid = zk.pull_user_records().expect("pull after push");
        assert_eq!(mid.len(), before.len(), "ممنوع إنشاء مستخدم مكرر");
        let t1 = mid
            .iter()
            .find(|(_, p, ..)| p == "9999")
            .expect("المستخدم اختفى بعد الإرسال!");
        println!("بعد الإرسال: uid={} priv={} name={:?}", t1.0, t1.4, t1.2);
        assert_eq!(t1.0, uid0, "يجب إعادة استخدام نفس uid");
        assert_eq!(t1.2, "اختبار عربي", "الاسم العربي يُخزَّن كما أُرسل (UTF-8)");
        assert_eq!(t1.4, priv0, "الحفاظ على الامتياز");
        assert_eq!(t1.3, card0, "الحفاظ على البطاقة");
        assert_eq!(t1.5, pw0, "الحفاظ على كلمة السر");

        zk.add_user("9999", &name0).expect("restore failed");
        let after = zk.pull_user_records().expect("pull after restore");
        assert_eq!(after.len(), before.len());
        let t2 = after
            .iter()
            .find(|(_, p, ..)| p == "9999")
            .expect("المستخدم اختفى بعد الاسترجاع!");
        assert_eq!(t2.2, name0, "استرجاع الاسم الأصلي");
        assert_eq!(t2.0, uid0);
        zk.close();
        println!("LIVE PUSH TEST OK");
    }
}
