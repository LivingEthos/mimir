class Mimir < Formula
  desc "Context-governed coding CLI for more accurate AI edits"
  homepage "https://github.com/LivingEthos/mimir"
  license :cannot_represent

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-aarch64-apple-darwin.tar.xz"
      sha256 "11fff039ebdce467d25b88727166ed0d659f5371e354a169643a4a6da722db8a"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-x86_64-apple-darwin.tar.xz"
      sha256 "13b2ab4d7f47e1e76a719e87bb64896c944441a75cef5a5d78ec6a5ead94d996"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "bafc80ac1ea718f0a667d35b9682b929e8222aafe5914ea671a210bac0fd582b"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "9c32100447e7ab62425107874eb41ea4722c6f578183b3cb63cd91ab360ba13d"
    end
  end

  def install
    bin.install "mimir"
  end

  test do
    assert_match "mimir #{version}", shell_output("#{bin}/mimir --version")
  end
end
