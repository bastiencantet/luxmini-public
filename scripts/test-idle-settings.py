#!/usr/bin/env python3
"""Exercise idle-event radio buttons and notifications with a memory light driver."""

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
    parser.add_argument("--screenshots", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="luxmini-idle-test-") as directory:
        contents = Path(directory) / "LuxMini Idle Test.app" / "Contents"
        executable = contents / "MacOS" / "IdleTest"
        executable.parent.mkdir(parents=True)
        shutil.copy2(args.binary.resolve(), executable)
        with (contents / "Info.plist").open("wb") as stream:
            plistlib.dump({
                "CFBundleIdentifier": "com.bastiencantet.luxmini.settings-test." + uuid.uuid4().hex,
                "CFBundleExecutable": executable.name,
                "CFBundleName": "LuxMini Idle Test",
                "CFBundlePackageType": "APPL",
                "NSHighResolutionCapable": True,
            }, stream)
        resources = contents / "Resources"
        assets = Path(__file__).resolve().parents[1] / "assets"
        resources.mkdir()
        for source in assets.glob("welcome-*.png"):
            shutil.copy2(source, resources / source.name)
        for source in assets.glob("welcome-*.jpg"):
            shutil.copy2(source, resources / source.name)
        environment = {key: value for key, value in os.environ.items() if not key.startswith("LUXMINI_")}
        environment["LUXMINI_CHECK_IDLE_SETTINGS"] = "1"
        if args.screenshots:
            environment["LUXMINI_IDLE_PREVIEW_DIR"] = str(args.screenshots.resolve())
        result = subprocess.run([str(executable)], env=environment, text=True, capture_output=True, timeout=30)
        print(result.stdout, end="")
        if result.returncode or "PASS: native idle radios" not in result.stdout:
            print(result.stderr, end="")
            raise SystemExit("Native idle-event verification failed")


if __name__ == "__main__":
    main()
