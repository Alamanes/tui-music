//! 系统媒体集成（macOS 26）。
//!
//! 让 macOS 把本播放器当作正常音乐播放器：播放时发布「正在播放」元数据
//! （控制中心 / 锁屏显示标题、艺术家、进度），并接收硬件媒体键 / AirPods
//! 的播放控制命令（播放 / 暂停 / 下一首 / 上一首）。
//!
//! # 关键事实（2026-08 在 macOS 26.5.1 实测 / 反汇编确认）
//!
//! - MediaRemote 是私有框架。macOS 15.4 起 `mediaremoted` 要求客户端进程有
//!   `com.apple.*` 的**代码签名标识**，否则客户端以 `entitlements=0` 被立即
//!   失效（日志：`Retrieved CFBundleIdentifier ... derived from code-signing
//!   information: bundleID=(null)`）。内嵌 `__info_plist` 无效——它读的是签名信息。
//!   解锁方式：构建后 ad-hoc 签名并指定标识：
//!   `codesign --force -s - --identifier com.apple.tui-music <binary>`
//!   （实测后 `mediaremoted` 以 `bundleIdentifier = com.apple.tui-music,
//!   entitlements=512` 接受，控制中心主动拉取本应用播放队列，播放状态上报为 Playing）。
//! - macOS 26 已移除旧版 `MRMediaRemoteRegisterCommandHandler` 与
//!   `MRMediaRemoteSetNowPlayingApplicationIsPlaying`；新版命令注册走
//!   `MRMediaRemoteSetCommandHandlerBlock`。
//! - `MRMediaRemoteSetCommandHandlerBlock` 的处理器 block 签名（反汇编确认）：
//!   `id (^)(unsigned int commandCode, NSDictionary *info)` —— **两个参数**，
//!   返回对象。旧资料里的 `NSInteger (^)(MRMediaRemoteCommandInfo *)` 单参形式
//!   在 26 上会被框架以 `(code, dict)` 调用，导致把命令码当指针解引用而崩溃。
//! - 播放状态经「正在播放」信息字典的 `PlaybackRate` 上报（rate=1 播放 / 0 暂停），
//!   配合 `MRMediaRemoteSetCanBeNowPlayingApplication` 声明身份。
//! - `MRMediaRemoteSetSupportedCommands` 在 26 上签名已改为 `*ForClient` 形式，
//!   直接调用会崩溃，本项目不使用（不影响播放控制）。
//!
//! 非 macOS 平台为空实现。

/// 从系统收到的一条远程媒体命令（AirPods / 硬件媒体键触发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Play,
    Pause,
    TogglePlayPause,
    Stop,
    NextTrack,
    PreviousTrack,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Command;
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::mem;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use objc2_foundation::{ns_string, NSDate, NSMutableDictionary, NSNumber, NSString, NSObject};

    /// MediaRemote 私有框架的命令码（`MRMediaRemoteCommand`）。
    const CMD_PLAY: u32 = 0;
    const CMD_PAUSE: u32 = 1;
    const CMD_TOGGLE_PLAY_PAUSE: u32 = 2;
    const CMD_STOP: u32 = 3;
    const CMD_NEXT_TRACK: u32 = 4;
    const CMD_PREVIOUS_TRACK: u32 = 5;

    /// 正在播放信息字典的键（私有但多年稳定）。
    const KEY_TITLE: &str = "kMRMediaRemoteNowPlayingInfoTitle";
    const KEY_ARTIST: &str = "kMRMediaRemoteNowPlayingInfoArtist";
    const KEY_DURATION: &str = "kMRMediaRemoteNowPlayingInfoDuration";
    const KEY_ELAPSED: &str = "kMRMediaRemoteNowPlayingInfoElapsedTime";
    const KEY_TIMESTAMP: &str = "kMRMediaRemoteNowPlayingInfoTimestamp";
    const KEY_RATE: &str = "kMRMediaRemoteNowPlayingInfoPlaybackRate";

    type FnSetNowPlayingInfo = unsafe extern "C" fn(*const c_void);
    type FnSetCanBeNowPlaying = unsafe extern "C" fn(bool);
    /// 新版命令处理器注册：传入一个 block（框架持有）。
    type FnSetCommandHandler = unsafe extern "C" fn(*const c_void);

    #[link(name = "System", kind = "dylib")]
    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        /// 泵主 run loop：MediaRemote 的命令投递走主线程 run loop / 主队列，
        /// TUI 主循环本身没有 Cocoa run loop，必须周期性调用它服务回调。
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: bool) -> c_int;
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecTaskCreateFromSelf(flags: u32) -> *mut c_void;
        fn SecTaskCopySigningIdentifier(task: *mut c_void, flags: u32, out: *mut *mut c_void) -> c_int;
        fn CFStringGetCString(s: *const c_void, buf: *mut c_char, bufsize: isize, encoding: u32) -> bool;
        fn CFRelease(cf: *const c_void);
    }

    /// 运行时解析 MediaRemote 符号；解析不到返回 `None`。
    macro_rules! resolve {
        ($handle:expr, $name:expr, $t:ty) => {{
            let cname = CString::new($name).ok()?;
            let sym = dlsym($handle, cname.as_ptr());
            if sym.is_null() {
                None
            } else {
                Some(mem::transmute::<*mut c_void, $t>(sym))
            }
        }};
    }

    /// 已解析的 MediaRemote 符号集合（缺失的为 `None`，优雅降级）。
    struct Api {
        set_now_playing: Option<FnSetNowPlayingInfo>,
        set_can_be_now_playing: Option<FnSetCanBeNowPlaying>,
        set_command_handler: Option<FnSetCommandHandler>,
    }

    impl Api {
        fn load() -> Option<Self> {
            // SAFETY: 合法 C 字符串；RTLD_NOW | RTLD_GLOBAL。
            // 进程常驻期刻意不 dlclose：MediaRemote 是全局单例框架，过早释放
            // 会使已注册的处理器/回调悬垂，泄漏由 OS 在退出时回收，代价可忽略。
            let handle = unsafe {
                dlopen(
                    c"/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote".as_ptr(),
                    0x2 | 0x8,
                )
            };
            if handle.is_null() {
                return None;
            }
            Some(Api {
                set_now_playing: unsafe {
                    resolve!(handle, "MRMediaRemoteSetNowPlayingInfo", FnSetNowPlayingInfo)
                },
                set_can_be_now_playing: unsafe {
                    resolve!(
                        handle,
                        "MRMediaRemoteSetCanBeNowPlayingApplication",
                        FnSetCanBeNowPlaying
                    )
                },
                set_command_handler: unsafe {
                    resolve!(handle, "MRMediaRemoteSetCommandHandlerBlock", FnSetCommandHandler)
                },
            })
        }
    }

    /// 把 MediaRemote 命令码映射到 [`Command`]；不认识的返回 `None`。
    pub(crate) fn map_command(code: u32) -> Option<Command> {
        match code {
            CMD_PLAY => Some(Command::Play),
            CMD_PAUSE => Some(Command::Pause),
            CMD_TOGGLE_PLAY_PAUSE => Some(Command::TogglePlayPause),
            CMD_STOP => Some(Command::Stop),
            CMD_NEXT_TRACK => Some(Command::NextTrack),
            CMD_PREVIOUS_TRACK => Some(Command::PreviousTrack),
            _ => None,
        }
    }

    /// 持有已注册的命令处理器 block；随 Bridge 存活至进程退出。
    struct Registration {
        api: Api,
        _block: RcBlock<dyn Fn(u32, *mut c_void) -> *mut c_void>,
    }

    impl Registration {
        fn register(tx: Sender<Command>) -> Option<Self> {
            let api = Api::load()?;
            let set = api.set_command_handler?;
            // SAFETY: 框架对 block 做自有拷贝（addCommandHandlerBlock:forKey: 持有），
            // 此处保留一份引用使 block 在进程生命周期内有效；TUI 主线程泵 run loop 时
            // 命令以 (commandCode, infoDict) 两个参数回调本 block。
            let block: RcBlock<dyn Fn(u32, *mut c_void) -> *mut c_void> =
                RcBlock::new(move |code: u32, _info: *mut c_void| -> *mut c_void {
                    if let Some(c) = map_command(code) {
                        let _ = tx.send(c);
                    }
                    // 返回 nil：框架对返回值做 msv_map: 映射后回传 completion；
                    // nil 是安全值（对 nil 发消息返回 nil），命令投递不受影响。
                    std::ptr::null_mut()
                });
            let block_ptr = &*block as *const _ as *const c_void;
            // SAFETY: 调用框架注册函数，把 block 交付给框架（框架持自有拷贝）。
            unsafe { set(block_ptr) };
            Some(Self { api, _block: block })
        }
    }

    /// 把正在播放元数据与播放状态发布到系统。
    fn publish(api: &Api, playing: bool, title: &str, artist: &str, duration: f64, elapsed: f64) {
        let dict = NSMutableDictionary::<NSString, NSObject>::new();
        dict.insert(ns_string!(KEY_TITLE), &*NSString::from_str(title));
        dict.insert(ns_string!(KEY_ARTIST), &*NSString::from_str(artist));
        dict.insert(ns_string!(KEY_DURATION), &*NSNumber::new_f64(duration));
        dict.insert(ns_string!(KEY_ELAPSED), &*NSNumber::new_f64(elapsed));
        dict.insert(ns_string!(KEY_TIMESTAMP), &*NSDate::new());
        dict.insert(
            ns_string!(KEY_RATE),
            &*NSNumber::new_f64(if playing { 1.0 } else { 0.0 }),
        );
        // SAFETY: NSDictionary 与 CFDictionary 免费桥接，指针仅在调用期间有效。
        unsafe {
            if let Some(f) = api.set_now_playing {
                f(&*dict as *const NSMutableDictionary<NSString, NSObject> as *const c_void);
            }
        }
    }

    /// 查询本进程的代码签名标识（用于提示用户签名解锁）。
    /// 返回 `None` 表示拿不到标识（未签名 / 非 macOS 26 语义）。
    fn signing_identifier() -> Option<String> {
        // SAFETY: 纯查询函数。errSecSuccess == 0。
        unsafe {
            let task = SecTaskCreateFromSelf(0);
            if task.is_null() {
                return None;
            }
            let mut out: *mut c_void = std::ptr::null_mut();
            let err = SecTaskCopySigningIdentifier(task, 0, &mut out);
            CFRelease(task);
            if err != 0 || out.is_null() {
                return None;
            }
            let mut buf = [0i8; 256];
            let ok = CFStringGetCString(out, buf.as_mut_ptr(), 256, 0x08000100 /* UTF-8 */);
            CFRelease(out);
            if !ok {
                return None;
            }
            let mut len = 0;
            while len < 256 && buf[len] != 0 {
                len += 1;
            }
            let bytes: Vec<u8> = buf[..len].iter().map(|&c| c as u8).collect();
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
    }

    /// 系统媒体集成是否可用：要求代码签名标识以 `com.apple.` 开头
    /// （mediaremoted 只接受这类客户端；`entitlements=512` 与系统平台二进制同级）。
    pub fn system_integration_available() -> bool {
        signing_identifier()
            .map(|id| id.starts_with("com.apple."))
            .unwrap_or(false)
    }

    struct Snapshot {
        playing: bool,
        title: String,
        artist: String,
        duration: f64,
    }

    pub struct Bridge {
        rx: Receiver<Command>,
        reg: Option<Registration>,
        /// 是否已向系统声明「可成为正在播放应用」。
        claimed: bool,
        last_pub: Instant,
        last: Option<Snapshot>,
    }

    impl Bridge {
        pub fn new() -> Self {
            let (tx, rx) = channel();
            let reg = Registration::register(tx);
            Self {
                rx,
                reg,
                claimed: false,
                last_pub: Instant::now() - Duration::from_secs(10),
                last: None,
            }
        }

        /// 取出一条待处理的远程命令；没有则返回 `None`。
        pub fn poll(&mut self) -> Option<Command> {
            self.rx.try_recv().ok()
        }

        /// 服务主 run loop（MediaRemote 命令回调依赖它）。
        /// 在 TUI 主循环每帧调用；无待处理事件时立即返回。
        pub fn pump(&mut self) {
            // SAFETY: NSString 与 CFString 免费桥接；mode 字符串常驻有效。
            let mode = NSString::from_str("kCFRunLoopDefaultMode");
            unsafe {
                CFRunLoopRunInMode(&*mode as *const NSString as *const c_void, 0.0, true);
            }
        }

        /// 发布当前播放状态。仅在真正播放时向系统声明身份；
        /// 无曲目（title 为空）时收回声明并清空卡片，避免残留与抢占。
        pub fn sync(&mut self, playing: bool, title: &str, artist: &str, duration: f64, elapsed: f64) {
            let has_track = !title.is_empty();
            if has_track && playing && !self.claimed {
                if let Some(reg) = &self.reg {
                    if let Some(f) = reg.api.set_can_be_now_playing {
                        // SAFETY: 框架函数，bool 表示“可作为正在播放应用”。
                        unsafe { f(true) };
                    }
                }
                self.claimed = true;
            }
            if !has_track && self.claimed {
                if let Some(reg) = &self.reg {
                    if let Some(f) = reg.api.set_can_be_now_playing {
                        unsafe { f(false) };
                    }
                }
                self.claimed = false;
            }

            let now = Instant::now();
            let changed = self.last.as_ref().is_none_or(|l| {
                l.playing != playing
                    || l.title != title
                    || l.artist != artist
                    || l.duration != duration
            });
            let due = now.duration_since(self.last_pub) >= Duration::from_millis(500);
            if !changed && !due {
                return;
            }
            self.last = Some(Snapshot {
                playing,
                title: title.to_string(),
                artist: artist.to_string(),
                duration,
            });
            self.last_pub = now;
            if let Some(reg) = &self.reg {
                publish(&reg.api, playing && has_track, title, artist, duration, elapsed);
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::Bridge;

#[cfg(target_os = "macos")]
pub use imp::system_integration_available;

#[cfg(not(target_os = "macos"))]
pub struct Bridge;

#[cfg(not(target_os = "macos"))]
impl Bridge {
    pub fn new() -> Self {
        Self
    }
    pub fn poll(&mut self) -> Option<Command> {
        None
    }
    pub fn pump(&mut self) {}
    pub fn sync(&mut self, _playing: bool, _title: &str, _artist: &str, _duration: f64, _elapsed: f64) {}
}

#[cfg(not(target_os = "macos"))]
pub fn system_integration_available() -> bool {
    false
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::imp::map_command;
    use super::Command as C;
    use super::*;

    /// 覆盖六个已知命令码 + 一个未知码（应返回 None）。纯函数，无条件运行。
    #[test]
    fn map_command_handles_all_known_codes() {
        assert_eq!(map_command(0), Some(C::Play));
        assert_eq!(map_command(1), Some(C::Pause));
        assert_eq!(map_command(2), Some(C::TogglePlayPause));
        assert_eq!(map_command(3), Some(C::Stop));
        assert_eq!(map_command(4), Some(C::NextTrack));
        assert_eq!(map_command(5), Some(C::PreviousTrack));
        assert_eq!(map_command(9999), None);
    }
}
