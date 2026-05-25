class Mimir < Formula
  desc "Replayable Context for coding agents"
  homepage "https://mimir.dev"
  version "1.0.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-aarch64-apple-darwin.tar.xz"
      sha256 "5e82466333ce0c5d4003fa33ee8e67884f5b15d80242ca3c258125c33040c462"
    else
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-x86_64-apple-darwin.tar.xz"
      sha256 "96d132ec085d42198476ad9ded15f24b19c8193ee51e55bd45035911964ea56d"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "c4a3447a8a11d212b048b6bf8d0672423019f872bdc917e0c2c70940aa541471"
    else
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "4be25c93b976038f34acd3d70532cacd28f03db42d971b2ffdd4d785fb5e2d32"
    end
  end

  def install
    bin.install "mimir"
  end

  test do
    assert_match "mimir #{version}", shell_output("#{bin}/mimir --version")
  end
end
