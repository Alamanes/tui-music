#!/bin/sh
# 为 tui-music 应用 macOS 系统媒体集成所需的 ad-hoc 签名。
#
# macOS 15.4+ 的 mediaremoted 只接受代码签名标识以 com.apple. 开头的客户端，
# 未签名的进程访问 MediaRemote 会静默失效（控制中心不显示、媒体键不响应）。
#
# 用法：
#   ./scripts/sign.sh [/path/to/tui-music]
# 不传路径时默认签名 target/release/tui-music（从源码构建后的产物）。
set -e

BIN="${1:-$(dirname "$0")/../target/release/tui-music}"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

if [ ! -f "$BIN" ]; then
    echo "未找到可执行文件: $BIN" >&2
    echo "请先构建：cargo build --release" >&2
    exit 1
fi

codesign --force -s - --identifier com.apple.tui-music "$BIN"
echo "已签名: $BIN (identifier=com.apple.tui-music)"
