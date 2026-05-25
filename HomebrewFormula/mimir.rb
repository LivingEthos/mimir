class Mimir < Formula
  desc "Context-governed coding CLI for more accurate AI edits"
  homepage "https://github.com/LivingEthos/mimir"
  license :cannot_represent

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-aarch64-apple-darwin.tar.xz"
      sha256 "7d6e364ee6b77e0a141048d0dcb6a994c12b8813702f8daf34ca79d529e68db7"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-x86_64-apple-darwin.tar.xz"
      sha256 "c39216591bc8540ef60c68af490014870636696ed378518e1971743b3a6e4f64"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "2f0702bbac0a3f6ac5e8fd4b74d2d3528a27867b03d01f3979884219bb4dac59"
    else
      url "https://github.com/LivingEthos/mimir/releases/download/v1.0.0/mimir-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "528a17f761169eb013cd79d2429e5cf74db30c088aa87f1cd90acfb0fb6f3256"
    end
  end

  def install
    bin.install "mimir"
  end

  test do
    assert_match "mimir #{version}", shell_output("#{bin}/mimir --version")
  end
end
