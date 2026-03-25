fn main() {
    // Delay-load winfsp-x64.dll so the app starts without WinFSP installed.
    // The DLL is only loaded when mount_drive is actually called.
    winfsp::build::winfsp_link_delayload();
    tauri_build::build()
}
