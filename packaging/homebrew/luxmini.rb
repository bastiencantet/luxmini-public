# Draft Homebrew cask for a future `bastiencantet/homebrew-tap` repository.
cask "luxmini" do
  version "0.4.0"
  sha256 "dd2a3cc919820d88ccc5abfc9047db9422c75844584e4883b4d1ce1a07d4ced3"

  url "https://github.com/bastiencantet/luxmini-public/releases/download/v#{version}/LuxMini-#{version}.dmg"
  name "LuxMini"
  desc "Turn off, dim, or schedule your Mac's front LED"
  homepage "https://luxmini.bastiencantet.com"

  app "LuxMini.app"
end
