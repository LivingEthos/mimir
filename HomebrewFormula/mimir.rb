class Mimir < Formula
  desc "Replayable Context for coding agents"
  homepage "https://github.com/LivingEthos/mimir"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-aarch64-apple-darwin.tar.xz"
      sha256 "ef59d33092d67d12e023fc59a88980a41a84cbd1be768f79ed6329cc8f30a200"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-x86_64-apple-darwin.tar.xz"
      sha256 "9a5b5d4b71c36dc249c0f54244218b2f31c74e2b81c32b629f24c011cfe65c2f"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "727a07ca5aef125d53f01d732c9543b5dbb43a88abaf15782fd60bd87659cdaa"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "cc6e798e21e1b9908e2ea50801b6d84d817abd732e1a1898f08344aed6a7e8e4"
    end
  end

  def install
    bin.install "mimir"
  end

  test do
    assert_match "mimir #{version}", shell_output("#{bin}/mimir --version")
  end
end
