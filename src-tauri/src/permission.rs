//! 마이크 권한.
//!
//! macOS는 권한이 없는 앱에게 **입력 장치를 아예 숨긴다.** 그래서 권한을 따로
//! 요청하지 않으면 이런 교착에 빠진다.
//!
//! 1. 권한이 없으니 CoreAudio가 입력 채널을 0으로 보고한다
//! 2. cpal의 `input_devices()`가 `supports_input()`으로 거르므로 목록이 빈다
//! 3. 앱은 "마이크가 없다"고 판단해 녹음 버튼을 막는다
//! 4. 스트림을 열 일이 없으니 macOS가 권한 창을 띄우지 않는다
//! 5. 요청한 적이 없으므로 **시스템 설정의 마이크 목록에도 나타나지 않는다**
//!
//! `Info.plist`에 `NSMicrophoneUsageDescription`이 있어도 소용없다. 그건 창이
//! 뜰 때 보여줄 문구일 뿐, 창을 띄우지는 않는다. 장치를 훑기 전에 여기서 먼저
//! 권한을 요청해야 고리가 끊긴다.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicPermission {
    /// 쓸 수 있다.
    Granted,
    /// 아직 물어본 적이 없다. 요청하면 창이 뜬다.
    NotDetermined,
    /// 거부됐다. 창이 다시 뜨지 않으므로 시스템 설정으로 보내야 한다.
    Denied,
    /// 권한 개념이 없는 플랫폼.
    NotRequired,
}

impl MicPermission {
    /// 장치를 훑어봐도 되는 상태인지.
    pub fn usable(self) -> bool {
        matches!(self, MicPermission::Granted | MicPermission::NotRequired)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::MicPermission;
    use objc2_av_foundation::{
        AVAuthorizationStatus, AVCaptureDevice, AVMediaType, AVMediaTypeAudio,
    };

    /// 프레임워크가 제공하는 전역 상수. 링크만 되면 항상 값이 있다.
    fn media_type() -> &'static AVMediaType {
        unsafe { AVMediaTypeAudio }.expect("AVMediaTypeAudio 상수를 찾을 수 없습니다")
    }

    fn from_status(status: AVAuthorizationStatus) -> MicPermission {
        match status {
            AVAuthorizationStatus::Authorized => MicPermission::Granted,
            AVAuthorizationStatus::NotDetermined => MicPermission::NotDetermined,
            // Restricted는 사용자가 풀 수 없는 정책 제한이지만, 앱 입장에서 할
            // 수 있는 게 Denied와 같으므로 묶는다.
            _ => MicPermission::Denied,
        }
    }

    pub fn status() -> MicPermission {
        let s = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type()) };
        from_status(s)
    }

    /// 권한 창을 띄우고 사용자가 답할 때까지 기다린다.
    ///
    /// 완료 핸들러가 다른 스레드에서 불리므로 채널로 받는다. 이미 결정된
    /// 상태라면 macOS가 창 없이 즉시 답한다.
    pub fn request() -> MicPermission {
        let current = status();
        if current != MicPermission::NotDetermined {
            return current;
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let handler = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
            let _ = tx.send(granted.as_bool());
        });
        unsafe {
            AVCaptureDevice::requestAccessForMediaType_completionHandler(
                media_type(),
                &handler,
            );
        }

        // 사용자가 창을 그냥 두는 경우가 있다. 무한정 붙잡고 있으면 호출한
        // 쪽이 멈추므로 상한을 둔다 — 그때는 현재 상태를 그대로 돌려준다.
        match rx.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(true) => MicPermission::Granted,
            Ok(false) => MicPermission::Denied,
            Err(_) => status(),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::MicPermission;

    pub fn status() -> MicPermission {
        MicPermission::NotRequired
    }

    pub fn request() -> MicPermission {
        MicPermission::NotRequired
    }
}

/// 지금 권한 상태. 창을 띄우지 않는다.
pub fn status() -> MicPermission {
    imp::status()
}

/// 필요하면 권한 창을 띄우고 결과를 돌려준다.
pub fn request() -> MicPermission {
    imp::request()
}

/// 시스템 설정의 마이크 항목을 연다.
///
/// 한 번 거부하면 권한 창이 다시 뜨지 않는다. 사용자가 설정을 직접 찾아가게
/// 두는 대신 바로 열어준다.
pub fn open_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        const URL: &str =
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone";
        std::process::Command::new("open")
            .arg(URL)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("시스템 설정을 열 수 없습니다: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("이 플랫폼에는 마이크 권한 설정이 없습니다.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granted_is_usable() {
        assert!(MicPermission::Granted.usable());
        assert!(MicPermission::NotRequired.usable());
    }

    /// 거부·미결정 상태에서 장치를 훑으면 빈 목록이 나와서 "마이크 없음"으로
    /// 오해하게 된다. 훑기 전에 걸러내야 한다.
    #[test]
    fn undecided_and_denied_are_not_usable() {
        assert!(!MicPermission::Denied.usable());
        assert!(!MicPermission::NotDetermined.usable());
    }
}
