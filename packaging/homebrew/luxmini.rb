# Draft Homebrew cask — goes in a tap repo `bastiencantet/homebrew-tap`
# (`Casks/luxmini.rb`). Until builds are notarized, prefer `make run`
# (build-from-source = no Gatekeeper). Fill in version + sha256 per release.
cask "luxmini" do
  version "0.3.0"
  sha256 "REPLACE_WITH_RELEASE_DMG_SHA256"

  url "https://github.com/bastiencantet/luxmini-public/releases/download/v#{version}/LuxMini-#{version}.dmg"
  name "LuxMini"
  desc "Turn off, dim, or schedule your Mac's front LED"
  homepage "https://luxmini.bastiencantet.com"

  app "LuxMini.app"

  caveats <<~EOS
    LuxMini is not notarized yet. On first launch:
      right-click LuxMini.app → Open → Open
    or:  xattr -d com.apple.quarantine "#{appdir}/LuxMini.app"
  EOS
end
