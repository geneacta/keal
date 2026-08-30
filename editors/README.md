# Editors

Highlighting, snippets and file icons for Keal. One grammar serves every
editor here: [`vscode/syntaxes/keal.tmLanguage.json`](vscode/syntaxes/keal.tmLanguage.json)
is a plain TextMate grammar, which is the format VS Code, JetBrains IDEs,
Sublime Text and Zed all read.

## VS Code

**The quickest way — install it locally.** VS Code loads every extension
folder it finds in `~/.vscode/extensions`, so a symlink is enough and it
keeps updating as the repository does:

```sh
# macOS / Linux
ln -s "$PWD/editors/vscode" ~/.vscode/extensions/keal
```

```powershell
# Windows (PowerShell, as administrator)
New-Item -ItemType SymbolicLink -Path "$env:USERPROFILE\.vscode\extensions\keal" -Target "$PWD\editors\vscode"
```

Restart VS Code (or run **Developer: Reload Window** from the command
palette). Open any `.keal` file: the status bar should say **Keal**. If it
says *Plain Text*, click it and pick Keal, which also teaches VS Code the
association for good.

**As a package**, if you would rather install a real `.vsix` — or hand one
to somebody else:

```sh
cd editors/vscode
npx --yes @vscode/vsce package        # writes keal-0.2.0.vsix
code --install-extension keal-0.2.0.vsix
```

Publishing it to the Marketplace needs a publisher account and a token;
nothing in the extension prevents it, and the manifest is already filled
in.

**What you get:** highlighting for the whole language (including `weak`,
`deinit`, `try`/`catch`/`throw` and the eight connectives), bracket and
comment behaviour, and the snippets in [`vscode/snippets`](vscode/snippets).

**Diagnostics inline.** The extension carries no language server yet, so
errors appear where you ask for them rather than as you type. A task that
runs the checker over the open file is the whole of it:

```jsonc
// .vscode/tasks.json
{
  "version": "2.0.0",
  "tasks": [{
    "label": "keal check",
    "type": "shell",
    "command": "keal check ${file}",
    "problemMatcher": {
      "owner": "keal",
      "fileLocation": ["relative", "${workspaceFolder}"],
      "pattern": {
        "regexp": "^\\s*--> (.*):(\\d+):(\\d+)$",
        "file": 1, "line": 2, "column": 3
      }
    },
    "group": "build"
  }]
}
```

## JetBrains IDEs (IntelliJ IDEA, CLion, PyCharm, …)

JetBrains IDEs read TextMate bundles natively, and they accept a VS Code
extension folder as one — so the same directory works, with no plugin to
write:

1. **Settings → Editor → TextMate Bundles**
2. **+** (Add), and choose the `editors/vscode` folder in this repository
3. Apply, then reopen any `.keal` file

Highlighting, brackets and comments follow immediately. If the IDE does
not associate the extension by itself, add it under **Settings → Editor →
File Types → TextMate** with the pattern `*.keal`.

Colours come from your IDE theme's TextMate mapping rather than from a
Keal-specific palette, so they will match whatever theme you already use.

What a TextMate bundle cannot give you is structural editing — go to
definition, rename, completion. That needs a language server, which is
the honest next step for editor support in general (one server would
serve VS Code, JetBrains, Neovim and Zed at once) and is not written yet.

## File icons

[`assets/keal-file.svg`](../assets/keal-file.svg) is the document icon for
`.keal` files. VS Code only lets an **icon theme** contribute file icons,
not a language extension, so using it means either adding it to an icon
theme you maintain, or setting it per-project in an editor that allows it.
JetBrains IDEs take it under **Settings → Appearance → File Types** where
a custom type can carry an icon.

## Other editors

* **Sublime Text** — the `.tmLanguage.json` can be converted with
  `PackageDev`, or dropped in as-is in recent builds.
* **Zed** — extensions want a Tree-sitter grammar rather than TextMate;
  none is written yet.
* **Neovim** — same: Tree-sitter, not written yet.
* **Anything else that reads TextMate** — point it at the same file.
