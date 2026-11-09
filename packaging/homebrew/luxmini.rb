# Draft Homebrew cask for a future `bastiencantet/homebrew-tap` repository.
cask "luxmini" do
  version "0.5.0"
  sha256 "2524ae60f43f9e24a03ac5833d22474ef210018dac8d1ae276f08b7fe6a99953"

  url "https://github.com/bastiencantet/luxmini-public/releases/download/v#{version}/LuxMini-#{version}.dmg"
  name "LuxMini"
  desc "Turn off, dim, or schedule your Mac's front LED"
  homepage "https://luxmini.bastiencantet.com"

  app "LuxMini.app"
end
