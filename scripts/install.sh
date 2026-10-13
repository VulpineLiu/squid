#!/usr/bin/env bash
# 把 squid 装进 PATH,并处理 macOS 的隔离属性。
#
#   ./scripts/install.sh                      # 用 cargo build --release 的产物
#   ./scripts/install.sh ./dist/squid         # 装一个已经下载/构建好的二进制
#   PREFIX=~/.local/bin ./scripts/install.sh  # 装到自己目录(不需要 sudo)
#
# 装完之后 `squid` 就是一条 PATH 里的普通命令 —— 装到 /usr/local/bin 或
# /opt/homebrew/bin 这种本来就在 PATH 上的目录,不需要再改 shell 配置。
set -euo pipefail

cd "$(dirname "$0")/.."
NAME=squid
BIN=${1:-target/release/$NAME}

pick_prefix() {
  if [ -n "${PREFIX:-}" ]; then
    printf '%s' "$PREFIX"
  elif [ -w /opt/homebrew/bin ]; then
    printf '%s' /opt/homebrew/bin
  elif [ -w /usr/local/bin ]; then
    printf '%s' /usr/local/bin
  else
    printf '%s' "$HOME/.local/bin"
  fi
}

if [ ! -x "$BIN" ]; then
  printf '\033[1;36m==>\033[0m 没有 %s,先构建\n' "$BIN"
  cargo build --release -p squid-cli
fi

PREFIX_DIR="$(pick_prefix)"
if [ ! -w "$PREFIX_DIR" ] && [ ! -e "$PREFIX_DIR" ]; then
  mkdir -p "$PREFIX_DIR"
fi
TARGET="$PREFIX_DIR/$NAME"
if [ ! -w "$PREFIX_DIR" ]; then
  printf '\033[1;33m提示:\033[0m %s 不可写,用 sudo 装一次(不会装守护进程)\n' "$PREFIX_DIR"
  sudo install -m 0755 "$BIN" "$TARGET"
else
  install -m 0755 "$BIN" "$TARGET"
fi

# 从浏览器/网络拿到的文件带 com.apple.quarantine,Gatekeeper 会拦一下。
# 自己 curl 下来的 release 包同理,清掉它等价于「我知道这是我自己的二进制」。
xattr -d com.apple.quarantine "$TARGET" 2>/dev/null || true
if [ "$(uname -m)" = arm64 ]; then
  codesign --force --sign - "$TARGET" >/dev/null 2>&1 || true
fi

printf '\033[1;32m已安装\033[0m %s\n' "$TARGET"
"$TARGET" --version || true
case ":$PATH:" in
  *":$PREFIX_DIR:"*) ;;
  *)
    printf '\033[1;33m注意:\033[0m %s 不在 PATH 里,把这一行加到 ~/.zshrc:\n' "$PREFIX_DIR"
    printf '  export PATH="%s:$PATH"\n' "$PREFIX_DIR"
    ;;
esac
