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
`deinit`, `try`/`catch`/`throw`, `constexpr`, `macro` and its `name!(...)`
call form, and the eight connectives), bracket and comment behaviour, and the
snippets in [`vscode/snippets`](vscode/snippets).

**A language server.** `keal lsp` speaks the Language Server Protocol over
stdin and stdout, so one binary serves every editor that speaks it. It gives
diagnostics as you type, the type of the thing under the cursor, go to
definition, find references, rename, an outline, and completion of the names
in scope.

It is not a second implementation of the language: it loads and checks the
file the way `keal check` does, reading the editor's unsaved buffer instead
of the disk. A wrong answer here would be a wrong answer in the compiler.

### Neovim

Built-in client, no plugin needed:

```lua
vim.filetype.add({ extension = { keal = "keal" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "keal",
  callback = function(args)
    vim.lsp.start({ name = "keal", cmd = { "keal", "lsp" },
                    root_dir = vim.fs.dirname(args.file) })
  end,
})
```

### Helix

```toml
# languages.toml
[language-server.keal]
command = "keal"
args = ["lsp"]

[[language]]
name = "keal"
scope = "source.keal"
file-types = ["keal"]
roots = ["keal.toml"]
comment-token = "//"
indent = { tab-width = 4, unit = "    " }
language-servers = ["keal"]
```

### Zed

```json
// ~/.config/zed/settings.json
{ "lsp": { "keal": { "binary": { "path": "keal", "arguments": ["lsp"] } } } }
```

### VS Code

The extension starts the server itself. It needs its one npm dependency
installed first, which is the only build step in this directory:

```sh
cd editors/vscode && npm install
```

`keal.server` in your settings points at a different binary, which is what
you want while working on the compiler itself. Without the server the
extension still highlights; it says so once and carries on.

**Diagnostics without the server.** If you would rather not run one, a task
that runs the checker over the open file gives you errors where you ask for
them:

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
