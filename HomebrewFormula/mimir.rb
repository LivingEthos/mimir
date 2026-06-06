class Mimir < Formula
  desc "Replayable Context for coding agents"
  homepage "https://github.com/LivingEthos/mimir"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-aarch64-apple-darwin.tar.xz"
      sha256 "PLACEHOLDER_SHA256_AARCH64_APPLE"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-x86_64-apple-darwin.tar.xz"
      sha256 "PLACEHOLDER_SHA256_X86_64_APPLE"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "PLACEHOLDER_SHA256_AARCH64_LINUX"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.1.0/mimir-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "PLACEHOLDER_SHA256_X86_64_LINUX"
    end
  end

  def install
    bin.install "mimir"
  end

  test do
    assert_match "mimir #{version}", shell_output("#{bin}/mimir --version")
  end
end
