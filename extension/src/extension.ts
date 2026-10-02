import * as fs from 'fs'
import * as path from 'path'
import { ExtensionContext, extensions, window, workspace } from 'vscode'
import { LanguageClient, LanguageClientOptions, ServerOptions, TransportKind } from 'vscode-languageclient/node'

type Toggle = 'auto' | 'on' | 'off'

type Status = { message: string }

const BUNDLED: Record<string, string> = {
  'darwin-arm64': 'magic-formatter-darwin-arm64',
  'darwin-x64': 'magic-formatter-darwin-x64',
  'linux-x64': 'magic-formatter-linux-x64',
  'linux-arm64': 'magic-formatter-linux-arm64',
  'win32-x64': 'magic-formatter-windows-x64.exe',
}

const COMPETING_SERVERS: Record<string, string[]> = {
  rust: ['rust-lang.rust-analyzer'],
  swift: ['swiftlang.swift-vscode', 'sswg.swift-lang'],
}

let client: LanguageClient | undefined
let lastOptions = ''

const bundledBinaryPath = (ctx: ExtensionContext) => {
  const name = BUNDLED[`${process.platform}-${process.arch}`]
  if (!name) return undefined
  const p = ctx.asAbsolutePath(path.join('bin', name))
  return fs.existsSync(p) ? p : undefined
}

const isEnabled = (toggle: Toggle, competitors: string[]) =>
  toggle === 'on' || (toggle === 'auto' && !competitors.some((id) => extensions.getExtension(id)))

const initializationOptions = () => {
  const cfg = workspace.getConfiguration('magicFormatter')
  const languages = Object.fromEntries(
    Object.entries(COMPETING_SERVERS).map(([id, competitors]) => [
      id,
      {
        semanticTokens: isEnabled(cfg.get<Toggle>(`${id}.semanticHighlighting`, 'auto'), competitors),
        references: isEnabled(cfg.get<Toggle>(`${id}.references`, 'auto'), competitors),
      },
    ]),
  )
  return { rustfmtPath: cfg.get('rustfmtPath', ''), topcoatPath: cfg.get('topcoatPath', ''), languages }
}

const start = async (ctx: ExtensionContext) => {
  const binary = workspace.getConfiguration('magicFormatter').get<string>('binaryPath') || bundledBinaryPath(ctx)
  if (!binary) {
    window.showErrorMessage(
      `Magic Formatter: no bundled binary for ${process.platform}-${process.arch}. ` +
        'Set "magicFormatter.binaryPath" to a locally built binary.',
    )
    return
  }

  const serverOptions: ServerOptions = { command: binary, args: ['--lsp'], transport: TransportKind.stdio }
  const clientOptions: LanguageClientOptions = {
    documentSelector: Object.keys(COMPETING_SERVERS).map((language) => ({ language })),
    initializationOptions: () => {
      const options = initializationOptions()
      lastOptions = JSON.stringify(options)
      return options
    },
    synchronize: { fileEvents: workspace.createFileSystemWatcher('**/*.{rs,swift}') },
  }

  client = new LanguageClient('magicFormatter', 'Magic Formatter', serverOptions, clientOptions)
  client.onNotification('magicFormatter/status', ({ message }: Status) => {
    window.setStatusBarMessage(message, 5000)
  })
  await client.start()
}

const restart = async (ctx: ExtensionContext) => {
  await client?.stop()
  client = undefined
  await start(ctx)
}

export const activate = async (ctx: ExtensionContext) => {
  ctx.subscriptions.push(
    workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('magicFormatter')) restart(ctx)
    }),
    extensions.onDidChange(() => {
      if (JSON.stringify(initializationOptions()) !== lastOptions) restart(ctx)
    }),
  )
  await start(ctx)
}

export const deactivate = () => client?.stop()
