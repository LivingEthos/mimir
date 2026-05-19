class Mimir < Formula
  desc "Replayable Context for coding agents"
  homepage "https://mimir.dev"
  version "1.0.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/mimir/mimir/releases/download/v1.0.0/mimir-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_SHA256_AARCH64_APPLE"
    else
      url "https://github.com/mimir/mimir/releases/download/v1.0.0/mimir-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_SHA256_X86_64_APPLE"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/mimir/mimir/releases/download/v1.0.0/mimir-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_SHA256_AARCH64_LINUX"
    else
      url "https://github.com/mimir/mimir/releases/download/v1.0.0/mimir-x86_64-unknown-linux-gnu.tar.gz"
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
