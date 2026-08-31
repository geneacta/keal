# Keal for Visual Studio Code

Syntax highlighting, editing behaviour, snippets, and errors reported inline
from the compiler itself.

## Installing

The extension is not on the marketplace. Link it into your extensions folder
and restart the editor:

```sh
# macOS and Linux
ln -s "$(pwd)/editors/vscode" ~/.vscode/extensions/keal

# Windows (PowerShell, from the repository root)
New-Item -ItemType SymbolicLink -Path "$HOME\.vscode\extensions\keal" -Target "$PWD\editors\vscode"
```

Open any `.keal` file to check it took. A file starting with
`#!/usr/bin/env keal` is recognised even without the extension.

## Errors in the editor

The extension ships a problem matcher that reads `keal check` output, so
diagnostics land on the right line with their message and note. Add this to
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

## What it does not know

There is no language server, so there is no completion, no go-to-definition,
and no hover types. Highlighting is lexical: the grammar colours a capitalised
name as a type because that is the convention, not because it resolved it.

The problem matcher is the honest substitute — it is the real compiler's real
answer, just triggered by a keystroke rather than on every edit.

## Other editors

The grammar in `syntaxes/keal.tmLanguage.json` is a standard TextMate file,
which Sublime Text, Zed and several others read directly. Point them at it and
associate `.keal`; nothing in it is specific to VS Code.
