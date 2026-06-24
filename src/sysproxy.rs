use crate::error::AppError;

#[cfg(target_os = "windows")]
use std::process::Command;

#[cfg(target_os = "windows")]
fn refresh_proxy() -> Result<(), AppError> {
    let ps_script = r#"
$code = @'
using System;
using System.Runtime.InteropServices;
public class WinINet {
    [DllImport("wininet.dll")]
    public static extern bool InternetSetOption(IntPtr hInternet, int dwOption, IntPtr lpBuffer, int dwBufferLength);
    public const int INTERNET_OPTION_SETTINGS_CHANGED = 39;
    public const int INTERNET_OPTION_REFRESH = 37;
    public static void Refresh() {
        InternetSetOption(IntPtr.Zero, INTERNET_OPTION_SETTINGS_CHANGED, IntPtr.Zero, 0);
        InternetSetOption(IntPtr.Zero, INTERNET_OPTION_REFRESH, IntPtr.Zero, 0);
    }
}
'@
Add-Type -TypeDefinition $code
[WinINet]::Refresh()
"#;

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(ps_script)
        .output()
        .map_err(|e| AppError::Io(format!("Failed to execute powershell: {}", e)))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Io(format!("PowerShell error: {}", err)));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn enable_system_proxy(port: u16) -> Result<(), AppError> {
    let script = format!(
        r#"
        Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -Name ProxyEnable -Value 1
        Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -Name ProxyServer -Value "127.0.0.1:{}"
        Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -Name ProxyOverride -Value "<local>;localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*"
        "#,
        port
    );

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(&script)
        .output()
        .map_err(|e| AppError::Io(format!("Failed to execute powershell: {}", e)))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Io(format!("PowerShell error: {}", err)));
    }

    refresh_proxy()?;

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn disable_system_proxy() -> Result<(), AppError> {
    let script = r#"
    Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -Name ProxyEnable -Value 0
    "#;

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(script)
        .output()
        .map_err(|e| AppError::Io(format!("Failed to execute powershell: {}", e)))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Io(format!("PowerShell error: {}", err)));
    }

    refresh_proxy()?;

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn enable_auto_start() -> Result<(), AppError> {
    let current_exe = std::env::current_exe()
        .map_err(|e| AppError::Io(format!("Failed to get current exe path: {}", e)))?;
    let exe_str = current_exe.to_str()
        .ok_or_else(|| AppError::Io("Failed to convert path to string".to_string()))?;

    let script = format!(
        r#"Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name '2con_client' -Value '"{}"'"#,
        exe_str
    );

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(&script)
        .output()
        .map_err(|e| AppError::Io(format!("Failed to execute powershell: {}", e)))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Io(format!("PowerShell error: {}", err)));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn disable_auto_start() -> Result<(), AppError> {
    let script = r#"Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name '2con_client' -ErrorAction SilentlyContinue"#;

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-Command")
        .arg(script)
        .output()
        .map_err(|e| AppError::Io(format!("Failed to execute powershell: {}", e)))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Io(format!("PowerShell error: {}", err)));
    }

    Ok(())
}

// No-op stub implementations for non-Windows platforms
#[cfg(not(target_os = "windows"))]
pub fn enable_system_proxy(_port: u16) -> Result<(), AppError> {
    Err(AppError::Io("System proxy is unsupported on this platform.".to_string()))
}

#[cfg(not(target_os = "windows"))]
pub fn disable_system_proxy() -> Result<(), AppError> {
    Err(AppError::Io("System proxy is unsupported on this platform.".to_string()))
}

#[cfg(not(target_os = "windows"))]
pub fn enable_auto_start() -> Result<(), AppError> {
    Err(AppError::Io("Auto-start is unsupported on this platform.".to_string()))
}

#[cfg(not(target_os = "windows"))]
pub fn disable_auto_start() -> Result<(), AppError> {
    Err(AppError::Io("Auto-start is unsupported on this platform.".to_string()))
}
