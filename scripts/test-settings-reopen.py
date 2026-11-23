#!/usr/bin/env python3
"""Verify native settings focus using a disposable bundle and no hardware startup."""

import argparse
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="luxmini-settings-test-") as directory:
        contents = Path(directory) / "LuxMini Settings Test.app" / "Contents"
        executable = contents / "MacOS" / "SettingsTest"
        executable.parent.mkdir(parents=True)
        shutil.copy2(args.binary.resolve(), executable)
        with (contents / "Info.plist").open("wb") as stream:
            plistlib.dump({
                "CFBundleIdentifier": "com.bastiencantet.luxmini.settings-test." + uuid.uuid4().hex,
                "CFBundleExecutable": executable.name,
                "CFBundleName": "LuxMini Settings Test",
                "CFBundlePackageType": "APPL",
                "NSHighResolutionCapable": True,
            }, stream)
        environment = {
            key: value for key, value in os.environ.items()
            if not key.startswith("LUXMINI_")
        }
        environment["LUXMINI_CHECK_SETTINGS_REOPEN"] = "1"
        result = subprocess.run(
            [str(executable)], env=environment, text=True,
            capture_output=True, timeout=30,
        )
        print(result.stdout, end="")
        if result.returncode or "PASS: settings regain focus" not in result.stdout:
            print(result.stderr, end="")
            raise SystemExit("Native settings reopen verification failed")


if __name__ == "__main__":
    main()
