class ClaudeFlash < Formula
  desc "Screen flashes when Claude Code finishes, asks, needs approval or fails"
  homepage "https://github.com/anish-agr/claude-flash"
  url "https://github.com/anish-agr/claude-flash/releases/download/v2.2.0/claude-flash-macos-universal.tar.gz"
  sha256 "7204b9a1b14d6d419622a9aa642fc0e009122b2aeca1a56228ac54dc6c0523b8"
  license "MIT"

  depends_on :macos

  def install
    bin.install "flash", "flash-agent"
  end

  def caveats
    <<~EOS
      To add the Claude Code hooks and start the agent, now and at every login:
        flash install
      After `brew upgrade claude-flash`, restart the agent:
        flash agent restart
      Before `brew uninstall claude-flash`, remove the hooks and the login item:
        flash uninstall
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/flash --version")
  end
end
