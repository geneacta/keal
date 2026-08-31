// The client half: it starts `keal lsp` and lets VS Code do the rest.
//
// Everything the extension knows about Keal beyond colour comes down this
// pipe, which is the point of a language server — the same binary answers
// Neovim, Zed and Helix with no second implementation to keep in step.

const { workspace, window } = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  // `keal` on the PATH by default; `keal.server` names another binary, which
  // is what you want when you are working on the compiler itself.
  const command = workspace.getConfiguration("keal").get("server") || "keal";
  const server = { command, args: ["lsp"], transport: TransportKind.stdio };

  client = new LanguageClient(
    "keal",
    "Keal",
    { run: server, debug: server },
    { documentSelector: [{ scheme: "file", language: "keal" }] }
  );

  client.start().catch((err) => {
    window.showWarningMessage(
      `Keal: could not start \`${command} lsp\` (${err.message}). ` +
        "Highlighting still works; set `keal.server` to the binary's path."
    );
  });
  context.subscriptions.push(client);
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
