-- LuxMini — control the Mac front LED from AppleScript.
--
-- Uses the `luxmini` CLI (which talks to LuxMini's local API). Requires the
-- LuxMini app running with the local API enabled (Settings > General).
-- Adjust the path if `luxmini` lives elsewhere (`which luxmini`).

on luxmini(args)
	return do shell script "/usr/local/bin/luxmini " & args
end luxmini

-- Examples (run any of these):
luxmini("off")
luxmini("on")
luxmini("brightness 128")
luxmini("effect blink")
luxmini("stop")
luxmini("get")
