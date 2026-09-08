#!/bin/bash
#
# Package this extension and let VS Code install it.
#
# Linking the folder into ~/.vscode/extensions by hand does not work: VS Code
# treats `extensions.json` as the record of what is installed, and a folder that
# appears on disk without a matching entry — symlink or not — is read as a
# leftover. It writes the id into `.obsolete` and drops the extension from its
# scan, grammar included, which reaches you as a .keal file with no colour at
# all rather than as an error.
#
# Going through a .vsix means VS Code writes that entry itself. The cost is that
# the installed extension is a copy: re-run this after editing the grammar.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

code="${VSCODE_BIN:-}"
if [ -z "$code" ]; then
  for candidate in \
    "$(command -v code || true)" \
    "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"; do
    [ -n "$candidate" ] && [ -x "$candidate" ] && code="$candidate" && break
  done
fi
if [ -z "$code" ]; then
  echo "No \`code\` command found. Install it from VS Code's command palette," >&2
  echo "'Shell Command: Install code command in PATH', or set VSCODE_BIN." >&2
  exit 1
fi

version="$(python3 -c "import json; print(json.load(open('$here/package.json'))['version'])")"
vsix="$(mktemp -d)/keal-$version.vsix"

npx --yes @vscode/vsce package --allow-missing-repository --skip-license -o "$vsix"
"$code" --install-extension "$vsix" --force

echo
echo "Installed keal $version. Restart VS Code, then open a .keal file."
echo "If it is still uncoloured, check the language indicator in the status bar:"
echo "the extension is from an unverified publisher, and VS Code may be holding"
echo "it until you trust the publisher in the Extensions view."
