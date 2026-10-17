// SPDX-License-Identifier: Apache-2.0

/**
 * Loading a DSL project through the TypeScript compiler API: the program over the project's
 * files (and virtual files, for tests), the checker, and the diagnostics the compiler reports.
 */

import path from 'node:path'

import ts from 'typescript'

/** A rule a module breaks, or a construct outside the subset, with where it is. */
export interface Diagnostic {
  file: string
  line: number
  column: number
  message: string
}

/** `file:line:column: message`. */
export function formatDiagnostic(diagnostic: Diagnostic): string {
  return `${diagnostic.file}:${diagnostic.line}:${diagnostic.column}: ${diagnostic.message}`
}

/** Thrown while lowering a construct the subset does not have, or that breaks a rule. */
export class CompileError extends Error {
  readonly node: ts.Node

  constructor(node: ts.Node, message: string) {
    super(message)
    this.node = node
  }
}

/** Where a node is, for diagnostics. */
export function locate(node: ts.Node): Pick<Diagnostic, 'file' | 'line' | 'column'> {
  const file = node.getSourceFile()
  const { line, character } = file.getLineAndCharacterOfPosition(node.getStart(file))
  return { file: file.fileName, line: line + 1, column: character + 1 }
}

/** The diagnostic for an error thrown at `node`. */
export function diagnose(error: CompileError): Diagnostic {
  return { ...locate(error.node), message: error.message }
}

/** A loaded DSL project. */
export interface Project {
  program: ts.Program
  checker: ts.TypeChecker
  /** The DSL's declarations, `uiharu/dsl/src/index.ts`. */
  lib: ts.SourceFile
  /** The directory of the project's `tsconfig.json`. */
  root: string
}

/**
 * Loads the project described by `configPath` (`uiharu/dsl/tsconfig.json`), with `virtual`
 * files — path to source — added to it, for tests that compile snippets.
 */
export function loadProject(configPath: string, virtual: Record<string, string> = {}): Project {
  const host: ts.ParseConfigFileHost = {
    ...ts.sys,
    onUnRecoverableConfigFileDiagnostic: (diagnostic) => {
      throw new Error(ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n'))
    },
  }
  const parsed = ts.getParsedCommandLineOfConfigFile(configPath, {}, host)
  if (!parsed) throw new Error(`${configPath} could not be read`)
  const root = path.dirname(path.resolve(configPath))
  const virtualFiles = new Map(
    Object.entries(virtual).map(([file, text]) => [path.resolve(file), text]),
  )
  const compilerHost = ts.createCompilerHost(parsed.options, true)
  const fileExists = compilerHost.fileExists.bind(compilerHost)
  const readFile = compilerHost.readFile.bind(compilerHost)
  const getSourceFile = compilerHost.getSourceFile.bind(compilerHost)
  compilerHost.fileExists = (file) => virtualFiles.has(path.resolve(file)) || fileExists(file)
  compilerHost.readFile = (file) => virtualFiles.get(path.resolve(file)) ?? readFile(file)
  compilerHost.getSourceFile = (file, languageVersion, onError, shouldCreate) => {
    const text = virtualFiles.get(path.resolve(file))
    if (text !== undefined) return ts.createSourceFile(file, text, languageVersion, true)
    return getSourceFile(file, languageVersion, onError, shouldCreate)
  }
  const program = ts.createProgram({
    rootNames: [...parsed.fileNames, ...virtualFiles.keys()],
    options: parsed.options,
    host: compilerHost,
  })
  const lib = program.getSourceFile(path.join(root, 'src', 'index.ts'))
  if (!lib) throw new Error(`${configPath} does not include src/index.ts`)
  return { program, checker: program.getTypeChecker(), lib, root }
}

/** TypeScript's own errors in `file`: a module with any is not lowered. */
export function typeErrors(project: Project, file: ts.SourceFile): Diagnostic[] {
  return [
    ...project.program.getSyntacticDiagnostics(file),
    ...project.program.getSemanticDiagnostics(file),
  ]
    .filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error)
    .map((diagnostic) => {
      const message = ts.flattenDiagnosticMessageText(diagnostic.messageText, '\n')
      if (diagnostic.file && diagnostic.start !== undefined) {
        const { line, character } = diagnostic.file.getLineAndCharacterOfPosition(diagnostic.start)
        return { file: diagnostic.file.fileName, line: line + 1, column: character + 1, message }
      }
      return { file: file.fileName, line: 0, column: 0, message }
    })
}

/** The modules of the project: every source file under `contracts/` (contracts of decoders
 * written by hand) and `decoders/`. */
export function moduleFiles(project: Project): ts.SourceFile[] {
  const dirs = ['contracts', 'decoders'].map((dir) => path.join(project.root, dir) + path.sep)
  return project.program
    .getSourceFiles()
    .filter((file) => dirs.some((dir) => path.resolve(file.fileName).startsWith(dir)))
    .sort((a, b) => a.fileName.localeCompare(b.fileName))
}
