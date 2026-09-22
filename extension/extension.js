const path = require('path')
const fs = require('fs')
const os = require('os')
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

const findUp = (dir, name) => {
  for (let d = dir; ; d = path.dirname(d)) {
    if (fs.existsSync(path.join(d, name))) return d
    if (path.dirname(d) === d) return null
  }
}

const topcoatBinary = (configured) => {
  if (configured) return configured
  const cargoBin = path.join(os.homedir(), '.cargo', 'bin', process.platform === 'win32' ? 'topcoat.exe' : 'topcoat')
  return fs.existsSync(cargoBin) ? cargoBin : 'topcoat'
}

const run = (binary, args, input, options) =>
  new Promise((resolve, reject) => {
    const child = execFile(binary, args, { ...options, maxBuffer: 64 * 1024 * 1024 }, (e, stdout, stderr) => {
      if (e) reject(new Error((stderr || e.message).split('\n')[0]))
      else resolve(stdout)
    })
    child.stdin.write(input)
    child.stdin.end()
  })

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

  const topcoatPath = cfg.get('topcoatPath') || ''

  const provider = {
    provideDocumentFormattingEdits: async (doc) => {
      const folder = vscode.workspace.getWorkspaceFolder(doc.uri)
      const cwd = folder ? folder.uri.fsPath : path.dirname(doc.uri.fsPath)
      let text
      try {
        text = await run(binary, [], doc.getText(), { cwd, env })
      } catch (e) {
        vscode.window.setStatusBarMessage(`Rustfmt Magic: ${e.message}`, 5000)
        return []
      }

      const topcoatRoot = findUp(path.dirname(doc.uri.fsPath), 'Topcoat.toml')
      if (topcoatRoot) {
        try {
          text = await run(topcoatBinary(topcoatPath), ['fmt', '--stdin'], text, { cwd: topcoatRoot })
        } catch (e) {
          vscode.window.setStatusBarMessage(`Rustfmt Magic (topcoat fmt): ${e.message}`, 5000)
        }
      }

      return [vscode.TextEdit.replace(fullRange(doc), text)]
    },
  }

  ctx.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider({ scheme: 'file', language: 'rust' }, provider),
  )
}

exports.deactivate = () => undefined
