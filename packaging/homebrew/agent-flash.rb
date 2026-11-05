class AgentFlash < Formula
  desc "Screen flashes when your coding agent finishes, asks, needs approval or fails"
  homepage "https://github.com/anish-agr/agent-flash"
  url "https://github.com/anish-agr/agent-flash/releases/download/v2.2.0/agent-flash-macos-universal.tar.gz"
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
      After `brew upgrade agent-flash`, restart the agent:
        flash agent restart
      Before `brew uninstall agent-flash`, remove the hooks and the login item:
        flash uninstall
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/flash --version")
  end
end
