class Mimir < Formula
  desc "Replayable Context for coding agents"
  homepage "https://mimir.dev"
  version "1.0.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-aarch64-apple-darwin.tar.xz"
      sha256 "773ad7851001cf1725bf50db5fd15697c2754a82c09c7f168412035b3ef188c4"
    else
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-x86_64-apple-darwin.tar.xz"
      sha256 "2cf9f5e5eb23af7b57b7b08c73b81aeddaccfcb4d6ea1ad9fd1a0dd7ca5bb3e7"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "bf2300a5e89cab374b856710326a51853459de820cd8a7eb8906b74b768ad6da"
    else
      url "https://github.com/MisterWonderful/mimir/releases/download/v1.0.0/mimir-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "4d022aafe48a965fc3b945b46b0138e5e0867fab9e128977502be0b102de0c18"
    end
  end

  def install
    bin.install "mimir"
  end

  test do
    assert_match "mimir #{version}", shell_output("#{bin}/mimir --version")
  end
end
