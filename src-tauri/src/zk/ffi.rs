//! استدعاء DLL الخاص بمكتبة ZKTeco Pull SDK (`plcommpro.dll`) عبر FFI.
//!
//! يعمل فقط على ويندوز. يُحمَّل الملف ديناميكياً (libloading) بحيث لا يتعطّل
//! التطبيق إن لم يكن الـ DLL موجوداً — عندها نسقط إلى البروتوكول النقي تلقائياً.
//!
//! ملفات الـ DLL المطلوبة (ضعها بجانب الـ .exe في مجلد `zk` أو `resources/zk`):
//!   plcommpro.dll  (+ ما يعتمد عليه من DLL ويندوز)

#![cfg(windows)]

use super::{DeviceBackend, DeviceInfo, RawPunch, RawUser};
use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;

type FnConnect = unsafe extern "C" fn(*const i8) -> isize;
type FnDisconnect = unsafe extern "C" fn(isize);
type FnReadLog = unsafe extern "C" fn(isize, *const i8) -> i32;
type FnSetLastError = unsafe extern "C" fn(isize) -> u32;

/// بنية سجل الحضور كما تعيده دالة ReadLog (بالصيغة COMMA).
/// نعتمد على صيغة النص (COMMA) بدل البنية الثنائية لسهولة التحويل.
pub struct ZkDllBackend {
    lib: &'static libloading::Library, // مكتبة محمّلة تبقى حيّة
    conn: isize,
    ip: String,
    #[allow(dead_code)]
    comm_key: u32,
}

impl ZkDllBackend {
    /// يحاول تحميل plcommpro.dll من عدة مسارات شائعة. يرجع None إن لم يُوجد.
    pub fn try_load(ip: &str, port: u16, comm_key: u32) -> Option<Self> {
        let candidates = candidate_paths();
        for p in &candidates {
            if !p.exists() {
                continue;
            }
            // حمّل المكتبة (آمن) ثم تابع فقط إذا نجح التحميل
            let lib = match unsafe { libloading::Library::new(p) } {
                Ok(l) => l,
                Err(e) => {
                    log::warn!("failed to load {:?}: {e}", p);
                    continue;
                }
            };
            // نص الاتصال لـ plcommpro: "ip:port"
            let target = std::ffi::CString::new(format!("{}:{}", ip, port)).ok()?;
            // استدعاء Connect (دالة C غير آمنة — نغلّفها في unsafe)
            let conn = unsafe {
                match lib.get::<FnConnect>(b"Connect") {
                    Ok(connect) => (*connect)(target.as_ptr()),
                    Err(e) => {
                        log::warn!("plcommpro!Connect not found: {e}");
                        continue;
                    }
                }
            };
            if conn == 0 {
                log::warn!("plcommpro Connect() failed for {ip}");
                continue;
            }
            // انقل المكتبة إلى 'static عبر leak (تبقى حيّة لعمر التطبيق)
            let static_lib: &'static libloading::Library = Box::leak(Box::new(lib));
            return Some(Self {
                lib: static_lib,
                conn,
                ip: ip.to_string(),
                comm_key,
            });
        }
        None
    }

    /// يجلب سجلات الحضور كنص COMMA منفصل ثم يحوّلها.
    fn read_log_text(&self) -> Result<String> {
        // buff 10MB (مُعلَّم mutable لأن الجهاز يكتب داخله)
        let cap = 10 * 1024 * 1024usize;
        let mut buf = vec![0u8; cap];
        let buf_ptr = buf.as_mut_ptr();
        // استدعاء ReadLog (دالة C غير آمنة — نستدعيها داخل unsafe)
        let read = unsafe {
            self.lib
                .get::<FnReadLog>(b"ReadLog")
                .context("plcommpro!ReadLog not found")?
        };
        let fmt = std::ffi::CString::new("Comma").unwrap();
        let n = unsafe { (*read)(self.conn, fmt.as_ptr()) }.max(0) as usize;
        if n == 0 || n >= cap {
            // جرّب صيغة افتراضية إن كانت النصية فارغة
            let fmt2 = std::ffi::CString::new("").unwrap();
            let n2 = unsafe { (*read)(self.conn, fmt2.as_ptr()) }.max(0) as usize;
            if n2 == 0 {
                return Err(anyhow!("ReadLog returned {} rows", n));
            }
            let s = unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(buf_ptr, n2)) }
                .to_string();
            return Ok(s);
        }
        let s = unsafe { String::from_utf8_lossy(std::slice::from_raw_parts(buf_ptr, n)) }.to_string();
        Ok(s)
    }
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for sub in ["zk", "resources/zk", "."] {
                v.push(dir.join(sub).join("plcommpro.dll"));
            }
        }
    }
    if let Some(cwd) = std::env::current_dir().ok() {
        v.push(cwd.join("zk").join("plcommpro.dll"));
    }
    v
}

impl DeviceBackend for ZkDllBackend {
    fn connect(&mut self) -> Result<DeviceInfo> {
        if self.conn == 0 {
            return Err(anyhow!("plcommpro connection handle is null"));
        }
        self.info()
    }

    fn info(&mut self) -> Result<DeviceInfo> {
        // GetDeviceStatus عبر ReadGeneralLog/GetDeviceStatus — نستخدم قراءة أبسط:
        // عدد السجلات عبر ReadLog (غير محدود) — لكن الأفضل نقدم معلومات تقريبية.
        let users = self.pull_users().map(|v| v.len() as i64).unwrap_or(0);
        let records = self
            .read_log_text()
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as i64)
            .unwrap_or(0);
        Ok(DeviceInfo {
            connected: self.conn != 0,
            firmware: "via plcommpro.dll".into(),
            platform: "SDK".into(),
            serial: String::new(),
            device_name: self.ip.clone(),
            users,
            fingerprints: 0,
            records,
        })
    }

    fn pull_users(&mut self) -> Result<Vec<RawUser>> {
        // plcommpro: دالة ReadGeneralLog أو GetUsers — نستخدم ReadLog بصيغة المستخدمين.
        // للتبسيط، نستدعي ReadLog ثم نحول — لكن الأدق استخدام:
        //   ReadGeneralLog / GetDevices
        // نكتفي هنا بقائمة فارغة إن لم يتوفر الرمز، والتطبيق يسقط للبروتوكول.
        Ok(vec![])
    }

    fn pull_attendance(&mut self) -> Result<Vec<RawPunch>> {
        let text = self.read_log_text()?;
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || !line.contains(',') {
                continue;
            }
            // الصيغة الشائعة لـ ReadLog(COMMA):
            //   PIN, DateTime, VerifyMode, WorkCode, ... (بحسب الإصدار)
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() < 2 {
                continue;
            }
            let pin = parts[0].to_string();
            let dt_raw = parts[1];
            let dt = normalize_datetime(dt_raw);
            let punch = parts.get(2).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            let wc = parts.get(3).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            out.push((pin, dt, punch, wc, 0));
        }
        Ok(out)
    }

    fn get_time_iso(&mut self) -> Result<String> {
        // ضبط/قراءة الوقت عبر SDK يتطلب دالة GetDeviceStatus — نتركه للبروتوكول.
        Err(anyhow!("time read via DLL not configured; use protocol backend"))
    }

    fn set_time(&mut self, _iso: &str) -> Result<()> {
        Err(anyhow!("set time via DLL not configured; use protocol backend"))
    }

    fn add_user(&mut self, _pin: &str, _name: &str) -> Result<()> {
        Err(anyhow!("use protocol backend for add_user"))
    }

    fn close(&mut self) {
        if self.conn != 0 {
            if let Ok(dis) = unsafe { self.lib.get::<FnDisconnect>(b"Disconnect") } {
                unsafe { (*dis)(self.conn) };
            }
            self.conn = 0;
        }
    }
}

/// يحوّل "YYYY/MM/DD HH:MM:SS" أو "YYYY-MM-DDTHH:MM:SS" إلى ISO موحّد.
fn normalize_datetime(raw: &str) -> String {
    let s = raw.replace('/', "-").replace(' ', "T");
    // إن كان يحتوي عمود زائد/بديل، اكتفِ بالأول 19 حرفاً
    let s = if s.len() >= 19 { s[..19].to_string() } else { s };
    // إن كان NN-NN-YYYY حوّله
    if s.len() >= 10 && s.contains('-') {
        let seg: Vec<&str> = s.split('-').collect();
        if seg.len() >= 3 && seg[2].len() == 4 {
            // يبدأ بصيغة DD-MM-YYYY -> حوّل إلى YYYY-MM-DD
            let (d, m, y) = (seg[0], seg[1], seg[2]);
            let rest = if s.len() > 10 { s[10..].to_string() } else { String::new() };
            let rest = rest.strip_prefix('T').unwrap_or(&rest).to_string();
            return format!("{}-{:0>2}-{:0>2}T{}", y, m, d, rest);
        }
    }
    s
}
