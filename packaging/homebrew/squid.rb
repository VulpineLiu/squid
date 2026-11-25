# Homebrew formula 模板。把整个文件放进你自己的 tap 仓库:
#
#   brew tap yourname/homebrew-tools https://github.com/yourname/homebrew-tools
#   brew install yourname/tools/squid
#
# 发布流程见 scripts/release.sh —— 它打出 tar.gz 并给出 sha256,填进下面两处
# 占位符即可。通用二进制(arm64 + x86_64)让同一个包在 Apple Silicon 和
# Intel 上都能装,bottle 交给 `brew tap-new` 的 CI 去做。
class Squid < Formula
  desc "Clean, uninstall, purge and watch a Mac from the terminal"
  homepage "https://github.com/VulpineLiu/squid"
  version "0.1.11"
  sha256 "REPLACE_WITH_DIST_SQUID_TARBALL_SHA256"
  url "https://github.com/VulpineLiu/squid/releases/download/v#{version}/squid-#{version}-universal-apple-darwin.tar.gz"
  license "GPL-3.0-only"

  # 也可以直接从源码装(不需要预先发 release):
  #   depends_on "rust" => :build
  #   def install
  #     system "cargo", "install", "--locked", "--root", prefix, "--path", "crates/squid-cli"
  #   end
  # 但那条路要求 tap 指向 git 仓库,而且每次 brew bump 都要重新编译。

  def install
    bin.install "squid-#{version}-universal-apple-darwin/squid" => "squid"
    generate_completions_from_executable(bin, "squid")
  end

  def caveats
    <<~EOS
      #{bin}/squid 已经在 PATH 上,直接 `squid` 打开界面。
      它不会安装任何后台进程,也不会改你的 shell 配置。
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/squid --version")
  end
end
