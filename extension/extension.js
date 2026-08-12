const path = require('path')
const fs = require('fs')
const vscode = require('vscode')
const { execFile } = require('child_process')

const BUNDLED = {
  'darwin-arm64': 'rustfmt-magic-darwin-arm64',
  'darwin-x64': 'rustfmt-magic-darwin-x64',
  'linux-x64': 'rustfmt-magic-linux-x64',
  'linux-arm64': 'rustfmt-magic-linux-arm64',
  'win32-x64': 'rustfmt-magic-windows-x64.exe',
}

const bundledBinaryPath = (ctx) => {
  const name = BUNDLED[`${process.platform}-${process.arch}`]
  if (!name) return null
  const p = ctx.asAbsolutePath(path.join('bin', name))
  return fs.existsSync(p) ? p : null
}

const fullRange = (doc) => new vscode.Range(doc.positionAt(0), doc.positionAt(doc.getText().length))

exports.activate = (ctx) => {
  const cfg = vscode.workspace.getConfiguration('rustfmtMagic')
  const binary = cfg.get('binaryPath') || bundledBinaryPath(ctx)
  if (!binary) {
    vscode.window.showErrorMessage(
      `Rustfmt Magic: no bundled binary for ${process.platform}-${process.arch}. ` +
        'Set "rustfmtMagic.binaryPath" to a locally built binary.',
    )
    return
  }
  const rustfmtPath = cfg.get('rustfmtPath') || ''
  const env = rustfmtPath ? { ...process.env, RUSTFMT_MAGIC_RUSTFMT: rustfmtPath } : process.env

  const provider = {
    provideDocumentFormattingEdits: (doc) =>
      new Promise((resolve) => {
        const folder = vscode.workspace.getWorkspaceFolder(doc.uri)
        const cwd = folder ? folder.uri.fsPath : path.dirname(doc.uri.fsPath)
        const child = execFile(binary, [], { cwd, env, maxBuffer: 64 * 1024 * 1024 }, (e, stdout, stderr) => {
          if (e) {
            if (stderr) vscode.window.setStatusBarMessage(`Rustfmt Magic: ${stderr.split('\n')[0]}`, 5000)
            resolve([])
          } else {
            resolve([vscode.TextEdit.replace(fullRange(doc), stdout)])
          }
        })
        child.stdin.write(doc.getText())
        child.stdin.end()
      }),
  }

  ctx.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider({ scheme: 'file', language: 'rust' }, provider),
  )
}

exports.deactivate = () => undefined
