class Hmm < Formula
  desc "Keyboard-centric terminal mind-map tool"
  homepage "https://github.com/your-org/hmm-rs"
  url "https://github.com/your-org/hmm-rs/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_RELEASE_TARBALL_SHA256"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "Error", shell_output("#{bin}/hmm --definitely-invalid-arg 2>&1", 1)
  end
end
