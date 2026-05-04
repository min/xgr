class Xgr < Formula
  desc "Rust implementation of XcodeGen-compatible project generation"
  homepage "https://github.com/min/xgr"
  url "https://github.com/min/xgr.git", branch: "main"
  version "0.1.0"
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
