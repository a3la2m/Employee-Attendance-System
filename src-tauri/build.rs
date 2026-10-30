fn main() {
    // مهم: عندما تُعاد بنية الواجهة (npm run build) تتغيّر ملفات dist (أسماء الهاش)،
    // لذا يجب أن يعاد تشغيل سكربت البناء كي يُضمِّن tauri الملفات الجديدة داخل
    // الملف التنفيذي — وإلا يبقى الملف القديم يحاول الاتصال بمخدم التطوير
    // localhost:1420 ويظهر «Could not connect to localhost: Connection refused».
    println!("cargo:rerun-if-changed=../dist");
    println!("cargo:rerun-if-changed=../src");
    tauri_build::build()
}
