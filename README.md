# tui-music

一个用 Rust 编写的终端音乐播放器，扫描 `~/Music` 下的音频文件并以漂亮的 TUI 列表呈现，附带实时频谱可视化与即时查找。

## 功能

- 自动扫描音乐目录（mp3 / flac / wav / ogg / m4a / aac / opus / aiff / alac）
- 读取标签（标题 / 艺术家 / 时长），无标签时回退到文件名
- 播放控制（state）：暂停 / 播放
- 切换曲目（next / prev）：下一首、上一首
- 循环模式（repeat）：List 列表循环 / One 单曲循环 / Off 关闭
- 随机播放（shuffle）：On / Off
- 音量调节（volume）：±5%
- 快进 / 快退（seek）：`h`/`←` 后退 5 秒，`l`/`→` 前进 5 秒
- 歌单（playlist）：自动识别 `~/Music` 下目录名含「歌单」的子目录，目录内歌曲归入对应命名歌单；其余歌曲进入默认「歌曲架」按原逻辑平铺，`[` / `]` 切换歌单
- 实时频谱（spectrum）：FFT 2048 → 每列一个密集小点（无块），自基线向上，宽度填满（不丢最右），对数频率分布，柔和渐变色，激进 gamma 压缩 + attack/release 平滑
- 即时查找（search）：键入字符实时过滤歌名 / 艺术家
- 会话记忆（state）：退出时保存音量 / 循环 / 随机 / 歌单与选中曲目到 `~/.config/tui-music/state.conf`，下次启动自动恢复（不自动播放）
- 进度条与当前曲目显示
- 系统媒体集成（macOS）：AirPods / 妙控键盘 / 硬件媒体键可控制播放（播放 / 暂停 / 下一首 / 上一首），并自动把当前曲目发布到系统「正在播放」，控制中心与锁屏可显示歌曲与进度

## 安装

### 通过 Homebrew（推荐，macOS / Linuxbrew）

本项目收录在作者个人 tap `orangerin/apps`，任何人都可直接安装：

```sh
brew tap orangerin/apps
brew install tui-music
```

或一行不 tap 直接装：

```sh
brew install orangerin/apps/tui-music
```

arm64 macOS 用户会自动下载预编译 bottle，无需本地编译；其他平台会从源码 `cargo build`。

升级：

```sh
brew upgrade tui-music
```

### 从源码构建

需要 Rust 工具链（`rustup` 或 Homebrew 的 `rust`）。

```sh
cargo build --release
# 二进制位于 target/release/tui-music
```

### 直接下载预编译二进制

到 [Releases](https://github.com/Shiorangerin/tui-music/releases) 下载对应平台的二进制，放入 `PATH` 即可。

## 使用

```sh
tui-music                                  # 扫描 ~/Music
tui-music -m /path/to/dir                  # 指定其他目录
./target/release/tui-music                 # 从源码构建后运行
```

## 快捷键

| 键           | 功能                |
| ------------ | ------------------- |
| `f` / `/`    | 进入查找模式        |
| 查找模式：键入 | 实时过滤歌名/艺术家 |
| 查找模式：`Enter` | 保留过滤退出     |
| 查找模式：`Esc`    | 清除过滤退出     |
| 查找模式：`↑` / `↓` | 在过滤结果中移动选择 |
| `j` / ↓      | 下移选择            |
| `k` / ↑      | 上移选择            |
| `Enter` | 播放选中曲目       |
| `h` / ← | 快退 5 秒 |
| `l` / → | 快进 5 秒 |
| `Space`      | 暂停 / 继续         |
| `n` / Tab    | 下一首              |
| `p`          | 上一首              |
| `r`          | 切换循环模式        |
| `s`          | 开关随机播放        |
| `+` / `=`    | 音量 +5%            |
| `-`          | 音量 -5%            |
| `[` / `]`    | 上一个 / 下一个歌单  |
| `q` / `Esc`  | 退出                |

## 系统媒体键 / AirPods（macOS）

在 macOS 上，播放器会注册为系统媒体会话：

- **AirPods 与硬件媒体键**：轻点/双击 AirPods 的播放键（播放 / 暂停 / 下一首 / 上一首）可直接控制 tui-music。
- **控制中心 / 锁屏**：正在播放的曲目、艺术家与进度会显示在系统「正在播放」卡片上。

实现基于系统（私有）MediaRemote 框架，通过 `dlsym` 在运行时解析符号、优雅降级（符号缺失时静默禁用，不影响播放本身）：

- **播放状态发布**：播放 / 暂停 / 切歌 / 停止时同步「正在播放」信息（标题 / 艺术家 / 时长 / 进度 / 播放速率），控制中心与锁屏据此渲染卡片；停止或退出后自动清空，不残留旧曲目。
- **命令路由**：使用新版 `MRMediaRemoteSetCommandHandlerBlock` 注册全局处理器，接收媒体键 / AirPods 命令并回传到播放器状态机。
- 只在真正播放时向系统声明媒体身份，未播放时不会抢占其他播放器的「正在播放」与媒体键。

> **macOS 15.4+ 需要签名**：系统只接受代码签名标识以 `com.apple.` 开头的进程访问媒体框架，未签名直接运行会**静默失效**（控制中心看不到、媒体键无效）。构建后请先签名（见下方「签名」一节），否则启动时会看到相应提示。

## 签名（macOS 15.4+ 必需）

从源码构建或下载二进制后，先执行一次签名：

```sh
codesign --force -s - --identifier com.apple.tui-music /path/to/tui-music
```

或使用仓库自带的脚本：

```sh
./scripts/sign.sh /path/to/tui-music
```

> 提示：媒体键 / AirPods 会路由到当前「正在播放」的应用；确认 tui-music 已被识别（控制中心能看到当前歌曲即说明成功）。若在旧版 macOS 上按键无效，请确认系统把 tui-music 视为当前媒体播放器。

## 歌单

播放器启动时扫描 `~/Music`（或 `-m` 指定目录）：

- 凡是**直接子目录**名包含「歌单」的目录，其内所有音频文件（含嵌套子目录）自动归入一个命名歌单，歌单名去掉「歌单」后缀（如 `日语歌单` → `日语`）。
- 不属于任何歌单目录的音频文件全部进入默认的「歌曲架」歌单，按原有逻辑平铺排序。
- 空的歌单目录会被忽略，不会出现在标签栏。
- 在 TUI 顶部的歌单标签栏按 `[` / `]` 切换；搜索仅在当前活跃歌单内过滤。

## 技术栈

- **ratatui** + **crossterm** — TUI 渲染与终端事件
- **rodio** — 基于 cpal 的跨平台音频播放与解码
- **rustfft** — 实时频谱 FFT
- **lofty** — 标签与音频属性读取
- **walkdir** — 递归扫描音乐目录
- **MediaRemote**（macOS 私有框架，运行时 `dlsym`）+ **objc2-foundation / block2** — 系统媒体键路由与「正在播放」发布

## 架构

```
src/
├── lib.rs        库入口（可测试）
├── main.rs        TUI 事件循环 & 终端管理 & 媒体键路由
├── app.rs         应用状态、键盘处理、查找过滤
├── ui.rs          ratatui 渲染（头部 / 查找栏 / 列表 / 频谱 / 状态栏）
├── library.rs    音乐目录扫描 & 标签读取  [库]
├── player.rs      rodio 播放封装 + VizBuffer 分流 [库]
├── viz.rs         FFT 频谱计算           [库]
├── state.rs       会话状态持久化          [库]
└── media.rs       macOS MediaRemote 桥接（AirPods 按键 + 正在播放）[bin, macOS]
```

播放器用 `MonoTee` 包装解码源：解码出的多声道样本下混为 mono，一边送往 cpal 播放，一边写入环形缓冲 `VizBuffer`。频谱模块从该缓冲取出最近的 2048 个采样，加汉宁窗后做 FFT，按对数频率聚合为终端宽度自适应条数；UI 以点字符自基线向上绘制。app 内对每个频段做 attack(0.7)/release(0.25) 帧间平滑，让起伏自然不抖。

## 测试

```sh
cargo test --release
```

包含音频扩展名识别与目录扫描两个集成测试。