# Keal for Visual Studio Code

Syntax highlighting, editing behaviour, snippets, and errors reported inline
from the compiler itself.

## Installing

The extension is not on the marketplace. Run the installer from the repository
root, then restart the editor:

```sh
./editors/vscode/install.sh
```

It links this folder into `~/.vscode/extensions` under the name VS Code gives
an installed extension, `geneacta.keal-<version>`. That name is the whole point
of the script. A link called plain `keal` works until the version in
`package.json` moves: VS Code then reads the folder as a stale install, writes
the id into `~/.vscode/extensions/.obsolete`, and drops the extension from its
scan — grammar included, which reaches you as a `.keal` file with no colour at
all rather than as an error. Re-run the installer after a version bump and the
name follows it.

On Windows, from the repository root in PowerShell:

```powershell
$v = (Get-Content editors\vscode\package.json | ConvertFrom-Json).version
New-Item -ItemType SymbolicLink -Path "$HOME\.vscode\extensions\geneacta.keal-$v" -Target "$PWD\editors\vscode"
```

Open any `.keal` file to check it took. A file starting with
`#!/usr/bin/env keal` is recognised even without the extension.

## Errors in the editor

The language server publishes diagnostics as you type, note and all, so in
the normal case there is nothing to run. The extension also ships a problem
matcher that reads `keal check` output, which is what you want when the server
is not running — you are rebuilding the compiler it points at, say. Add this to
your project's `.vscode/tasks.json` — this repository already has it:

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "keal: check file",
      "type": "shell",
      "command": "keal check ${relativeFile}",
      "problemMatcher": "$keal",
      "presentation": { "reveal": "silent" },
      "group": { "kind": "build", "isDefault": true }
    }
  ]
}
```

`Ctrl-Shift-B` (`Cmd-Shift-B`) then runs it, and the Problems panel fills with
whatever the checker found. Because Keal reports every independent error in
one pass, one run gives you the whole list rather than the first mistake.

## What it knows

* **Highlighting** for every keyword, including the eight logical connectives
  as words, the null-safety operators (`?.`, `?:`, `!!`) as their own colour,
  string interpolation as embedded code, and nested block comments.
* **Editing**: bracket matching, auto-closing, indentation, and `///` doc
  comments continued on the next line.
* **Snippets** for the declarations — type `func`, `proc`, `record`, `trait`,
  `when`, `unless`, `vald` for a destructuring binding, `shebang`.
* **A language server**, `keal lsp`, for hover types, go-to-definition,
  find-references, rename, document symbols, `.`-triggered completion, and the
  diagnostics above. One binary answers every editor that speaks the protocol,
  so there is no second implementation to keep in step.

## What it does not know

The server runs whatever `keal` is on your PATH. If that is not the binary you
mean — you are working on the compiler itself, or VS Code was started from the
Finder and never saw your shell's PATH — set `keal.server` to a full path:

```json
{ "keal.server": "/path/to/keal/target/release/keal" }
```

Without a server the extension still colours and still edits; you lose the
types and the navigation, and the extension says so once, in a notification.

Highlighting is lexical either way. The grammar colours a capitalised name as a
type because that is the convention, not because it resolved it.

## Other editors

The grammar in `syntaxes/keal.tmLanguage.json` is a standard TextMate file,
which Sublime Text, Zed and several others read directly. Point them at it and
associate `.keal`; nothing in it is specific to VS Code. `keal lsp` is likewise
an ordinary language server — Neovim, Zed and Helix each want a few lines of
their own configuration naming the binary, and get everything above.
