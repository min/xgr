class Xgr < Formula
  desc "Rust implementation of XcodeGen-compatible project generation"
  homepage "https://github.com/min/xgr"
  url "https://github.com/min/xgr/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "fb55aeb06ea92e693abb49d8f4e0655a16720e3b3e335abb31da95f8c877091f"
  license "MIT"

  head "https://github.com/min/xgr.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "Rust implementation of XcodeGen-compatible project.yml loading",
      shell_output("#{bin}/xgr --help")

    (testpath/"project.yml").write <<~YAML
      name: BrewSmokeTest
      targets: {}
    YAML

    assert_match "BrewSmokeTest", shell_output("#{bin}/xgr validate --spec project.yml")
  end
end
