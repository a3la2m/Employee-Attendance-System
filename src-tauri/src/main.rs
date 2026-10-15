//! ملف الإقلاع الفعلي (يُستخدم على ويندوز لأن حزمة الـ exe تبحث عن main).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    attendance_lib::run();
}
