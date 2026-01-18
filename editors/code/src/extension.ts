import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    Executable
} from 'vscode-languageclient/node';

let client: LanguageClient;

export function activate(context: vscode.ExtensionContext) {
    const config = vscode.workspace.getConfiguration('versionLsp');
    const serverPath = config.get<string>('serverPath') || 'version-lsp';

    const run: Executable = {
        command: serverPath,
        transport: { kind: 'stdio' },
    };

    const serverOptions: ServerOptions = {
        run,
        debug: run,
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'toml' },
            { scheme: 'file', language: 'json' },
            { scheme: 'file', language: 'jsonc' },
            { scheme: 'file', language: 'go.mod' },
            { scheme: 'file', language: 'yaml' },
            { scheme: 'file', language: 'python' },
        ],
    };

    client = new LanguageClient(
        'versionLsp',
        'Version LSP',
        serverOptions,
        clientOptions
    );

    client.start();
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined;
    }
    return client.stop();
}
