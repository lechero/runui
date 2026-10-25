class Runui < Formula
  desc "Project commands and persistent runs in your terminal"
  homepage "https://github.com/lechero/runui"
  url "https://github.com/lechero/runui.git",
      revision: "4c67b9c59de30c99ba5407d30f9b9b8f9b51dd23"
  version "0.1.0"
  license "MIT"

  head "https://github.com/lechero/runui.git", branch: "main"

  depends_on "rust" => :build

  def fetch
    system "cargo", "fetch", *std_cargo_fetch_args
  end

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/runui")
  end

  test do
    assert_match "runui keeps a folder", shell_output("#{bin}/runui help")
  end
end
