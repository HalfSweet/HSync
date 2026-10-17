// SPDX-License-Identifier: Apache-2.0

/**
 * Lowering a decoder module: its type declarations, contracts, constants and free functions,
 * the decoder definition (`defineLogicDecoder` …) with its class, and the contracts it reads
 * from other modules; then the passes that decide what is passed by mutable reference and
 * what borrows a field of the decoder.
 */

import path from 'node:path'

import ts from 'typescript'

import type {
  Const,
  Contract,
  Decoder,
  Expr,
  Framing,
  Function,
  Import,
  Json,
  Module,
  Param,
  PropSpec,
  RequireKind,
  RequireSpec,
  StateField,
  Stmt,
  Type,
  TypeDef,
} from './ir.ts'
import { Lowerer, type DecoderShape, type ModuleShape } from './lower.ts'
import { CompileError, diagnose, typeErrors, type Diagnostic, type Project } from './program.ts'
import { Types } from './types.ts'

/** The hooks whose parameters a method may leave out, with the parameters' names and types. */
const HOOKS: Record<string, Param[]> = {
  run: [],
  hold: [],
  onGap: [{ name: 'range', type: 'range' }],
  onEnd: [{ name: 'tick', type: 'u64' }],
  onProgress: [{ name: 'tick', type: 'u64' }],
}

const REQUIREMENTS: Record<string, RequireKind> = {
  atLeast: 'at-least',
  atMost: 'at-most',
  within: 'within',
  oneOf: 'one-of',
}

/** The result of lowering one module. */
export interface Lowered {
  module?: Module
  diagnostics: Diagnostic[]
}

/** Lowers the decoder module in `file`. */
export function lowerModule(project: Project, file: ts.SourceFile): Lowered {
  const errors = typeErrors(project, file)
  if (errors.length) return { diagnostics: errors }
  try {
    return { module: new ModuleLowerer(project, file).lower(), diagnostics: [] }
  } catch (error) {
    if (error instanceof CompileError) return { diagnostics: [diagnose(error)] }
    throw error
  }
}

/** What the definition object says. */
interface Definition {
  kind: 'logic' | 'stream' | 'bus'
  name: string
  category: string
  defaults: Json
  validate?: ts.Expression
  requires?: ts.Expression
  input?: ts.Expression
  output: ts.Expression
  props?: ts.Expression
  partial?: string
  recovery?: ts.ObjectLiteralExpression
  decoder: ts.Identifier
  node: ts.ObjectLiteralExpression
}

class ModuleLowerer {
  readonly types: Types
  readonly checker: ts.TypeChecker
  readonly typeDefs: TypeDef[]
  readonly functions = new Map<string, ts.FunctionDeclaration>()
  readonly classes = new Map<string, ts.ClassDeclaration>()
  readonly consts: Const[] = []
  readonly contracts: Contract[] = []
  readonly imports: Import[] = []
  definition?: Definition
  readonly shape: ModuleShape

  readonly project: Project
  readonly file: ts.SourceFile

  constructor(project: Project, file: ts.SourceFile) {
    this.project = project
    this.file = file
    this.checker = project.checker
    this.types = new Types(project.program, project.lib)
    this.typeDefs = this.types.collect(file)
    this.shape = {
      functions: new Set(),
      consts: new Set(),
      types: new Map(this.typeDefs.map((def) => [def.name, def])),
    }
  }

  lower(): Module {
    this.scan()
    const definition = this.definition
    if (!definition) throw new CompileError(this.file, 'a decoder module defines its decoder')
    const output = this.schemaOf(definition.output)
    const input = definition.input ? this.schemaOf(definition.input) : undefined
    const functions = this.freeFunctions()
    const decoder = this.decoder(definition, output.contract, input?.contract)
    if (definition.validate) {
      const validate = this.validate(definition.validate, decoder.config.type)
      if (typeof validate === 'string') decoder.config.validate = validate
      else {
        functions.push(validate)
        decoder.config.validate = validate.name
      }
    }
    this.mutableParams(functions, decoder.methods)
    for (const method of decoder.methods) this.markBorrows(method, decoder)
    const module: Module = { ir: 1, name: definition.name, decoder }
    if (this.typeDefs.length) module.types = this.typeDefs
    if (this.contracts.length) module.contracts = this.contracts
    if (this.imports.length) module.imports = this.imports
    if (this.consts.length) module.consts = this.consts
    if (functions.length) module.functions = functions
    // The decoder comes last in the JSON, after what it is made of.
    delete (module as Partial<Module>).decoder
    module.decoder = decoder
    return module
  }

  /** Reads the module's top-level declarations. */
  private scan() {
    for (const statement of this.file.statements) {
      if (ts.isFunctionDeclaration(statement) && statement.name) {
        this.functions.set(statement.name.text, statement)
        this.shape.functions.add(statement.name.text)
      } else if (ts.isClassDeclaration(statement) && statement.name) {
        this.classes.set(statement.name.text, statement)
      } else if (ts.isVariableStatement(statement)) {
        for (const decl of statement.declarationList.declarations) {
          this.topLevelVariable(decl, statement.declarationList.flags)
        }
      } else if (
        ts.isImportDeclaration(statement) ||
        ts.isTypeAliasDeclaration(statement) ||
        ts.isInterfaceDeclaration(statement) ||
        ts.isExportDeclaration(statement)
      ) {
        continue
      } else {
        throw new CompileError(
          statement,
          `${ts.SyntaxKind[statement.kind]} is not in the subset at the top of a module`,
        )
      }
    }
  }

  private topLevelVariable(decl: ts.VariableDeclaration, flags: ts.NodeFlags) {
    if (!ts.isIdentifier(decl.name) || !decl.initializer) {
      throw new CompileError(decl, 'a top-level declaration is a named constant')
    }
    if (!(flags & ts.NodeFlags.Const))
      throw new CompileError(decl, 'top-level declarations are const')
    const init = decl.initializer
    const contract = this.contractOf(init)
    if (contract) {
      this.contracts.push(contract)
      return
    }
    if (ts.isCallExpression(init) && ts.isIdentifier(init.expression)) {
      const lib = this.types.libName(init.expression)
      if (
        lib === 'defineLogicDecoder' ||
        lib === 'defineStreamDecoder' ||
        lib === 'defineBusDecoder'
      ) {
        if (this.definition) throw new CompileError(decl, 'a module defines one decoder')
        this.definition = this.readDefinition(init, lib)
        return
      }
    }
    // A constant: a literal with its type.
    const value = literalValue(init)
    if (value === undefined) {
      throw new CompileError(
        init,
        'a top-level constant is a literal, a schema or the decoder definition',
      )
    }
    if (!decl.type && typeof value !== 'boolean') {
      throw new CompileError(decl, 'an integer constant says its width')
    }
    const type: Type = decl.type ? this.types.mapNode(decl.type) : 'bool'
    this.consts.push({ name: decl.name.text, type, value })
    this.shape.consts.add(decl.name.text)
  }

  /** The contract an `eventSchema(...)` or `eventSchema(...).framed({...})` initializer declares. */
  private contractOf(init: ts.Expression): Contract | undefined {
    let call = init
    let framing: Framing | undefined
    if (
      ts.isCallExpression(call) &&
      ts.isPropertyAccessExpression(call.expression) &&
      call.expression.name.text === 'framed'
    ) {
      const arg = call.arguments[0]
      if (!arg || !ts.isObjectLiteralExpression(arg))
        throw new CompileError(call, 'framed takes an object')
      framing = this.framingOf(arg)
      call = call.expression.expression
    }
    if (!ts.isCallExpression(call) || !ts.isIdentifier(call.expression)) return undefined
    if (this.types.libName(call.expression) !== 'eventSchema') return undefined
    const typeArg = call.typeArguments?.[0]
    if (!typeArg || !ts.isTypeReferenceNode(typeArg) || !ts.isIdentifier(typeArg.typeName)) {
      throw new CompileError(call, 'eventSchema names its event type: eventSchema<Event>(…)')
    }
    const [name, version] = call.arguments
    if (!name || !ts.isStringLiteral(name) || !version || !ts.isNumericLiteral(version)) {
      throw new CompileError(call, 'eventSchema takes the name and the version as literals')
    }
    const contract: Contract = {
      schema: { name: name.text, version: Number(version.text) },
      event: typeArg.typeName.text,
    }
    if (framing) contract.framing = framing
    return contract
  }

  private framingOf(node: ts.ObjectLiteralExpression): Framing {
    const values = literalObject(node)
    const text = (key: string): string => {
      const value = values[key]
      if (typeof value !== 'string') throw new CompileError(node, `framing.${key} is a literal`)
      return value
    }
    const framing: Framing = {
      begin: text('begin'),
      item: text('item'),
      end: text('end'),
      id: text('id'),
      reason: text('reason'),
      normal: text('normal'),
    }
    if ('partial' in values) framing.partial = text('partial')
    if ('unframed' in values) {
      const unframed = values['unframed']
      if (typeof unframed !== 'number') throw new CompileError(node, 'framing.unframed is a number')
      framing.unframed = unframed
    }
    return framing
  }

  private readDefinition(call: ts.CallExpression, lib: string): Definition {
    const node = call.arguments[0]
    if (!node || !ts.isObjectLiteralExpression(node)) {
      throw new CompileError(call, `${lib} takes an object literal`)
    }
    const props = new Map<string, ts.Expression>()
    for (const property of node.properties) {
      if (ts.isPropertyAssignment(property) && ts.isIdentifier(property.name)) {
        props.set(property.name.text, property.initializer)
      } else if (ts.isShorthandPropertyAssignment(property)) {
        props.set(property.name.text, property.name)
      } else if (ts.isMethodDeclaration(property) && ts.isIdentifier(property.name)) {
        props.set(property.name.text, methodAsArrow(property))
      } else {
        throw new CompileError(property, 'the definition has named properties')
      }
    }
    const required = (key: string): ts.Expression => {
      const value = props.get(key)
      if (!value) throw new CompileError(node, `the definition needs ${key}`)
      return value
    }
    const textOf = (key: string): string => {
      const value = required(key)
      if (!ts.isStringLiteral(value)) throw new CompileError(value, `${key} is a literal`)
      return value.text
    }
    const config = required('config')
    if (!ts.isObjectLiteralExpression(config))
      throw new CompileError(config, 'config is an object literal')
    const configProps = new Map<string, ts.Expression>()
    for (const property of config.properties) {
      if (ts.isPropertyAssignment(property) && ts.isIdentifier(property.name)) {
        configProps.set(property.name.text, property.initializer)
      } else if (ts.isShorthandPropertyAssignment(property)) {
        configProps.set(property.name.text, property.name)
      } else if (ts.isMethodDeclaration(property) && ts.isIdentifier(property.name)) {
        configProps.set(property.name.text, methodAsArrow(property))
      }
    }
    const defaults = configProps.get('defaults')
    if (!defaults) throw new CompileError(config, 'config.defaults is required')
    const defaultsJson = jsonValue(defaults)
    if (defaultsJson === undefined)
      throw new CompileError(defaults, 'config.defaults is made of literals')
    const decoder = required('decoder')
    if (!ts.isIdentifier(decoder)) throw new CompileError(decoder, 'decoder names the class')
    const definition: Definition = {
      kind:
        lib === 'defineLogicDecoder' ? 'logic' : lib === 'defineStreamDecoder' ? 'stream' : 'bus',
      name: textOf('name'),
      category: textOf('category'),
      defaults: defaultsJson,
      output: required('output'),
      decoder,
      node,
    }
    const validate = configProps.get('validate')
    if (validate) definition.validate = validate
    const requires = props.get('requires')
    if (requires) definition.requires = requires
    const propsFn = props.get('props')
    if (propsFn) definition.props = propsFn
    if (definition.kind !== 'logic') definition.input = required('input')
    if (definition.kind === 'bus') {
      const partial = props.get('partial')
      if (partial) {
        if (!ts.isStringLiteral(partial)) throw new CompileError(partial, 'partial is a literal')
        definition.partial = partial.text
      }
      const recovery = required('recovery')
      if (!ts.isObjectLiteralExpression(recovery))
        throw new CompileError(recovery, 'recovery is an object literal')
      definition.recovery = recovery
    }
    return definition
  }

  /** The contract an identifier names, declared here or in another module (then imported). */
  private schemaOf(node: ts.Expression): { contract: Contract; foreign: boolean } {
    if (!ts.isIdentifier(node)) throw new CompileError(node, 'a schema is named by its constant')
    const symbol = this.types.symbolOf(node)
    const decl = symbol?.valueDeclaration
    if (!decl || !ts.isVariableDeclaration(decl) || !decl.initializer) {
      throw new CompileError(node, `${node.text} is not a schema`)
    }
    const contract = this.contractOf(decl.initializer)
    if (!contract) throw new CompileError(node, `${node.text} is not a schema`)
    const source = decl.getSourceFile()
    if (source === this.file) return { contract, foreign: false }
    this.importContract(source, contract)
    return { contract, foreign: true }
  }

  /** Imports a contract declared in `source`, with the types its events are made of. */
  private importContract(source: ts.SourceFile, contract: Contract) {
    if (this.imports.some((existing) => existing.contract.schema.name === contract.schema.name))
      return
    const defs = this.types.collect(source)
    const byName = new Map(defs.map((def) => [def.name, def]))
    const needed = new Set<string>()
    const visit = (name: string) => {
      const def = byName.get(name)
      if (!def || needed.has(name)) return
      needed.add(name)
      if (def.type === 'record') for (const field of def.fields) visitType(field.type)
      if (def.type === 'union') {
        for (const variant of def.variants)
          for (const field of variant.fields ?? []) visitType(field.type)
      }
    }
    const visitType = (type: Type) => {
      if (typeof type === 'string') visit(type)
      else if ('option' in type) visitType(type.option)
      else if ('array' in type) visitType(type.array)
      else visitType(type.batch)
    }
    visit(contract.event)
    const types = defs.filter((def) => needed.has(def.name))
    for (const def of types) this.shape.types.set(def.name, def)
    const entry: Import = { module: path.basename(source.fileName, '.ts'), contract }
    if (types.length) entry.types = types
    this.imports.push(entry)
  }

  private freeFunctions(): Function[] {
    const out: Function[] = []
    for (const [name, node] of this.functions) {
      if (node.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.AsyncKeyword)) {
        throw new CompileError(node, 'a free function is synchronous')
      }
      if (!node.body) throw new CompileError(node, 'a function has a body')
      const params = this.params(node.parameters)
      const lowerer = new Lowerer(this.types, this.shape)
      const body = lowerer.body(
        params.map((param) => param.name),
        node.body,
        { method: false, async: false },
      )
      const fn: Function = { name, body }
      if (params.length) fn.params = params
      const ret = this.returnType(node)
      if (ret !== undefined) fn.ret = ret
      out.push(fn)
    }
    return out
  }

  private params(nodes: readonly ts.ParameterDeclaration[]): Param[] {
    return nodes.map((node) => {
      if (!ts.isIdentifier(node.name)) throw new CompileError(node, 'a parameter has a plain name')
      if (!node.type) throw new CompileError(node, 'a parameter is typed')
      return { name: node.name.text, type: this.types.mapNode(node.type) }
    })
  }

  /** The return type of a function or method, `Promise<T>` unwrapped; `undefined` for none. */
  private returnType(node: ts.FunctionDeclaration | ts.MethodDeclaration): Type | undefined {
    let type: ts.Type
    if (node.type) type = this.checker.getTypeFromTypeNode(node.type)
    else {
      const signature = this.checker.getSignatureFromDeclaration(node)
      if (!signature) throw new CompileError(node, 'the return type could not be read')
      type = signature.getReturnType()
    }
    type = this.checker.getAwaitedType(type) ?? type
    if (type.flags & (ts.TypeFlags.Void | ts.TypeFlags.Undefined)) return undefined
    const mapped = this.types.map(type)
    if (mapped === undefined)
      throw new CompileError(node.type ?? node, 'the return type has no IR type')
    return mapped
  }

  /** `validate`: a function's name, or an arrow lifted to a free function. */
  private validate(node: ts.Expression, configType: string): string | Function {
    if (ts.isIdentifier(node)) {
      if (!this.functions.has(node.text))
        throw new CompileError(node, `${node.text} is not a function of this module`)
      return node.text
    }
    if (!ts.isArrowFunction(node) && !ts.isFunctionExpression(node)) {
      throw new CompileError(node, 'validate is a function')
    }
    if (this.functions.has('validate'))
      throw new CompileError(node, 'validate is already a function')
    const param = node.parameters[0]
    const name = param && ts.isIdentifier(param.name) ? param.name.text : 'config'
    const lowerer = new Lowerer(this.types, this.shape)
    const body = lowerer.body([name], node.body, { method: false, async: false })
    return {
      name: 'validate',
      params: [{ name, type: configType }],
      ret: { option: 'text' },
      body,
    }
  }

  /** The object literal an arrow (`(config) => ({ … })`) returns, with the parameter's name. */
  private arrowObject(
    node: ts.Expression,
    what: string,
  ): { param: string | undefined; object: ts.ObjectLiteralExpression } {
    if (!ts.isArrowFunction(node) && !ts.isFunctionExpression(node)) {
      throw new CompileError(node, `${what} is a function of the configuration`)
    }
    const param = node.parameters[0]
    const name = param && ts.isIdentifier(param.name) ? param.name.text : undefined
    let body: ts.Node = node.body
    if (ts.isBlock(body)) {
      const only = body.statements[0]
      if (
        body.statements.length !== 1 ||
        !only ||
        !ts.isReturnStatement(only) ||
        !only.expression
      ) {
        throw new CompileError(body, `${what} returns an object literal`)
      }
      body = only.expression
    }
    while (ts.isParenthesizedExpression(body)) body = body.expression
    if (!ts.isObjectLiteralExpression(body))
      throw new CompileError(body, `${what} returns an object literal`)
    return { param: name, object: body }
  }

  private requiresOf(node: ts.Expression): RequireSpec[] {
    const { param, object } = this.arrowObject(node, 'requires')
    const lowerer = new Lowerer(this.types, this.shape)
    const out: RequireSpec[] = []
    for (const property of object.properties) {
      if (!ts.isPropertyAssignment(property) || !ts.isIdentifier(property.name)) {
        throw new CompileError(property, 'a requirement is named')
      }
      const value = property.initializer
      let requirement: RequireKind = 'equals'
      let args: ts.Expression[] = [value]
      if (ts.isCallExpression(value) && ts.isIdentifier(value.expression)) {
        const lib = this.types.libName(value.expression)
        const kind = lib === undefined ? undefined : REQUIREMENTS[lib]
        if (kind) {
          requirement = kind
          args = [...value.arguments]
        }
      }
      out.push({
        key: property.name.text,
        requirement,
        values: args.map((arg) => lowerer.standalone(param ? [param] : [], arg, { method: false })),
      })
    }
    return out
  }

  private propsOf(node: ts.Expression): PropSpec[] {
    const { param, object } = this.arrowObject(node, 'props')
    const lowerer = new Lowerer(this.types, this.shape)
    const out: PropSpec[] = []
    for (const property of object.properties) {
      if (!ts.isPropertyAssignment(property) || !ts.isIdentifier(property.name)) {
        throw new CompileError(property, 'a property is named')
      }
      out.push({
        key: property.name.text,
        value: lowerer.standalone(param ? [param] : [], property.initializer, { method: false }),
      })
    }
    return out
  }

  private decoder(definition: Definition, output: Contract, input: Contract | undefined): Decoder {
    const cls = this.classes.get(definition.decoder.text)
    if (!cls)
      throw new CompileError(
        definition.decoder,
        `${definition.decoder.text} is not a class of this module`,
      )
    const base = cls.heritageClauses?.find(
      (clause) => clause.token === ts.SyntaxKind.ExtendsKeyword,
    )?.types[0]
    if (!base || !ts.isIdentifier(base.expression))
      throw new CompileError(cls, 'a decoder extends a decoder class')
    const baseName = this.types.libName(base.expression)
    const kinds: Record<string, 'logic' | 'stream' | 'bus'> = {
      LogicDecoder: 'logic',
      StreamDecoder: 'stream',
      BusDecoder: 'bus',
    }
    const kind = baseName === undefined ? undefined : kinds[baseName]
    if (!kind || kind !== definition.kind) {
      throw new CompileError(
        base,
        `the class of a ${definition.kind} decoder extends the matching base class`,
      )
    }
    const typeArgs = base.typeArguments ?? []
    const configArg = typeArgs[0]
    if (!configArg || !ts.isTypeReferenceNode(configArg) || !ts.isIdentifier(configArg.typeName)) {
      throw new CompileError(base, 'the first type argument names the configuration')
    }
    const configType = configArg.typeName.text
    if (this.shape.types.get(configType)?.type !== 'record') {
      throw new CompileError(configArg, `${configType} is not a record`)
    }
    const category = definition.category
    if (category !== 'bus' && category !== 'device' && category !== 'line') {
      throw new CompileError(definition.node, 'category is bus, device or line')
    }
    const decoder: Decoder = {
      name: definition.name,
      category,
      config: { type: configType, defaults: definition.defaults },
      input: kind === 'logic' ? { kind: 'logic' } : { kind, schema: (input as Contract).schema },
      output: { schema: output.schema },
      methods: [],
    }
    if (definition.requires) {
      const requires = this.requiresOf(definition.requires)
      if (requires.length) decoder.requires = requires
    }
    if (definition.props) {
      const props = this.propsOf(definition.props)
      if (props.length) decoder.props = props
    }
    if (kind === 'bus') {
      if (definition.partial === 'drain' || definition.partial === 'read')
        decoder.partial = definition.partial
      else if (definition.partial !== undefined)
        throw new CompileError(definition.node, 'partial is drain or read')
      const reasonArg = typeArgs[3]
      if (
        !reasonArg ||
        !ts.isTypeReferenceNode(reasonArg) ||
        !ts.isIdentifier(reasonArg.typeName)
      ) {
        throw new CompileError(base, 'the fourth type argument names the reason enum')
      }
      const values = literalObject(definition.recovery as ts.ObjectLiteralExpression)
      const reason = (key: string): string => {
        const value = values[key]
        if (typeof value !== 'string')
          throw new CompileError(definition.recovery as ts.Node, `recovery.${key} is a reason`)
        return value
      }
      decoder.recovery = {
        reason: reasonArg.typeName.text,
        incomplete: reason('incomplete'),
        truncated: reason('truncated'),
        missing: reason('missing'),
        gap: reason('gap'),
        end: reason('end'),
      }
    }
    // The class: its fields are the state, its methods the logic.
    const shape: DecoderShape = { kind, state: new Map(), methods: new Map() }
    const fields: ts.PropertyDeclaration[] = []
    const methods: ts.MethodDeclaration[] = []
    for (const member of cls.members) {
      if (ts.isPropertyDeclaration(member)) fields.push(member)
      else if (ts.isMethodDeclaration(member)) methods.push(member)
      else throw new CompileError(member, 'a decoder class has fields and methods only')
    }
    for (const method of methods) {
      if (!ts.isIdentifier(method.name)) throw new CompileError(method, 'a method has a plain name')
      shape.methods.set(
        method.name.text,
        method.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.AsyncKeyword) ?? false,
      )
    }
    const state: StateField[] = []
    for (const field of fields) {
      if (!ts.isIdentifier(field.name)) throw new CompileError(field, 'a field has a plain name')
      if (!field.initializer)
        throw new CompileError(field, 'a field is initialized where it is declared')
      const name = field.name.text
      const lowerer = new Lowerer(this.types, this.shape, shape)
      const init = lowerer.standalone([], field.initializer, { method: true })
      let type: Type
      if (field.type) type = this.types.mapNode(field.type)
      else {
        const mapped = this.types.map(this.checker.getTypeAtLocation(field.initializer))
        if (mapped === undefined) throw new CompileError(field, 'the field needs a type')
        type = mapped
      }
      if (typeof type === 'object' && 'batch' in type) {
        type = { batch: type.batch, bound: this.batchBound(field.initializer) }
      }
      const entry: StateField = { name, type, init }
      if (field.modifiers?.some((modifier) => modifier.kind === ts.SyntaxKind.ReadonlyKeyword)) {
        entry.readonly = true
      }
      state.push(entry)
      shape.state.set(name, type)
    }
    if (state.length) decoder.state = state
    for (const method of methods) {
      const name = (method.name as ts.Identifier).text
      if (!method.body) throw new CompileError(method, 'a method has a body')
      const isAsync = shape.methods.get(name) ?? false
      let params = this.params(method.parameters)
      const hook = HOOKS[name]
      if (hook && params.length < hook.length) params = [...params, ...hook.slice(params.length)]
      const lowerer = new Lowerer(this.types, this.shape, shape)
      const body = lowerer.body(
        params.map((param) => param.name),
        method.body,
        { method: true, async: isAsync },
      )
      const fn: Function = { name, body }
      if (isAsync) fn.async = true
      if (params.length) fn.params = params
      const ret = this.returnType(method)
      if (ret !== undefined) fn.ret = ret
      decoder.methods.push(fn)
    }
    return decoder
  }

  /** The capacity a `batch<T>(n)` initializer declares. */
  private batchBound(init: ts.Expression): number {
    let node = init
    while (ts.isParenthesizedExpression(node)) node = node.expression
    if (ts.isCallExpression(node) && node.arguments.length === 1) {
      const arg = node.arguments[0] as ts.Expression
      if (ts.isNumericLiteral(arg)) return Number(arg.text.replace(/_/g, ''))
      if (ts.isIdentifier(arg)) {
        const symbol = this.types.symbolOf(arg)
        const decl = symbol?.valueDeclaration
        if (
          decl &&
          ts.isVariableDeclaration(decl) &&
          decl.initializer &&
          ts.isNumericLiteral(decl.initializer)
        ) {
          return Number(decl.initializer.text.replace(/_/g, ''))
        }
      }
    }
    throw new CompileError(
      init,
      'a batch is made with batch<T>(capacity), the capacity a literal or a constant',
    )
  }

  // Passes

  /** Marks the parameters a function writes through, or passes to one that does. */
  private mutableParams(functions: Function[], methods: Function[]) {
    const all = [
      ...functions.map((fn) => ({ key: `fn:${fn.name}`, fn })),
      ...methods.map((fn) => ({ key: `method:${fn.name}`, fn })),
    ]
    const mutable = new Map<string, Set<string>>(all.map(({ key }) => [key, new Set()]))
    const callee = (expr: Expr): string | undefined =>
      expr.expr === 'call'
        ? `fn:${expr.name}`
        : expr.expr === 'method'
          ? `method:${expr.name}`
          : undefined
    let changed = true
    while (changed) {
      changed = false
      for (const { key, fn } of all) {
        const set = mutable.get(key) as Set<string>
        for (const param of fn.params ?? []) {
          if (set.has(param.name)) continue
          if (
            writesThrough(fn.body, param.name, (expr, index) => {
              const target = callee(expr)
              if (target === undefined) return false
              const params = all.find((entry) => entry.key === target)?.fn.params ?? []
              const name = params[index]?.name
              return name !== undefined && (mutable.get(target)?.has(name) ?? false)
            })
          ) {
            set.add(param.name)
            changed = true
          }
        }
      }
    }
    for (const { key, fn } of all) {
      const set = mutable.get(key) as Set<string>
      for (const param of fn.params ?? []) if (set.has(param.name)) param.mutable = true
    }
  }

  /** Turns a `let` of a record field of the decoder that is written through into a borrow,
   * and marks the bindings of such fields that are written through. */
  private markBorrows(fn: Function, decoder: Decoder) {
    const stateType = new Map((decoder.state ?? []).map((field) => [field.name, field.type]))
    const isRecordPlace = (expr: Expr): boolean => {
      const type = this.placeType(expr, stateType)
      return typeof type === 'string' && this.shape.types.get(type)?.type === 'record'
    }
    const visit = (block: Stmt[]) => {
      for (let i = 0; i < block.length; i++) {
        const stmt = block[i] as Stmt
        const rest = block.slice(i + 1)
        switch (stmt.stmt) {
          case 'let':
            if (
              isSelfPlace(stmt.value) &&
              isRecordPlace(stmt.value) &&
              writesThrough(rest, stmt.name, () => false)
            ) {
              block[i] = { stmt: 'let-ref', name: stmt.name, place: stmt.value }
            }
            break
          case 'if-some':
            if (
              isSelfPlace(stmt.value) &&
              isRecordPlace(stmt.value) &&
              writesThrough(stmt.then, stmt.bind, () => false)
            ) {
              stmt.borrow = true
            }
            visit(stmt.then)
            if (stmt.else) visit(stmt.else)
            break
          case 'let-some':
            if (
              isSelfPlace(stmt.value) &&
              isRecordPlace(stmt.value) &&
              writesThrough(rest, stmt.bind, () => false)
            ) {
              stmt.borrow = true
            }
            visit(stmt.else)
            break
          case 'if':
            visit(stmt.then)
            if (stmt.else) visit(stmt.else)
            break
          case 'match':
            for (const arm of stmt.arms) visit(arm.body)
            break
          case 'loop':
          case 'while':
          case 'for':
          case 'for-each':
            visit(stmt.body)
            break
          default:
            break
        }
      }
    }
    visit(fn.body)
  }

  /** The declared type of a place rooted at `self`, through fields; options are seen through. */
  private placeType(expr: Expr, state: Map<string, Type>): Type | undefined {
    const bare = (type: Type | undefined): Type | undefined =>
      typeof type === 'object' && type !== null && 'option' in type ? bare(type.option) : type
    if (expr.expr === 'self') return bare(state.get(expr.field))
    if (expr.expr === 'field') {
      const of = bare(this.placeType(expr.of, state))
      if (typeof of !== 'string') return undefined
      const def = this.shape.types.get(of)
      if (def?.type !== 'record') return undefined
      return bare(def.fields.find((field) => field.name === expr.name)?.type)
    }
    return undefined
  }
}

/** Whether `block` writes through the local `name`: assigns to a field or item of it, or
 * passes it where `passedMutably(call, index)` says it is written through. */
function writesThrough(
  block: Stmt[],
  name: string,
  passedMutably: (call: Expr, index: number) => boolean,
): boolean {
  const rooted = (expr: Expr): boolean => {
    if (expr.expr === 'field' || expr.expr === 'index') {
      const base = expr.of
      return (base.expr === 'local' && base.name === name) || rooted(base)
    }
    return false
  }
  const inExpr = (expr: Expr): boolean => {
    switch (expr.expr) {
      case 'call':
      case 'method':
        return (
          expr.args.some(
            (arg, index) => arg.expr === 'local' && arg.name === name && passedMutably(expr, index),
          ) || expr.args.some(inExpr)
        )
      case 'intrinsic':
        if (['batch-push', 'batch-take', 'batch-clear', 'mask-with'].includes(expr.name)) {
          const first = expr.args[0]
          if (first?.expr === 'local' && first.name === name && expr.name !== 'mask-with')
            return true
        }
        return expr.args.some(inExpr)
      case 'await':
        return inOp(expr.op)
      case 'field':
      case 'tag':
      case 'unary':
      case 'required':
      case 'cast':
        return inExpr('of' in expr ? expr.of : expr.value)
      case 'index':
        return inExpr(expr.of) || inExpr(expr.at)
      case 'binary':
        return inExpr(expr.left) || inExpr(expr.right)
      case 'record':
      case 'variant':
        return Object.values(expr.fields ?? {}).some(inExpr)
      case 'array':
      case 'format':
        return ('items' in expr ? expr.items : expr.parts).some(inExpr)
      case 'ternary':
        return inExpr(expr.cond) || inExpr(expr.then) || inExpr(expr.else)
      case 'if-some':
        return inExpr(expr.value) || inExpr(expr.some) || inExpr(expr.none)
      case 'if-variant':
        return inExpr(expr.value) || inExpr(expr.then) || inExpr(expr.else)
      default:
        return false
    }
  }
  const inOp = (op: { op: string } & Record<string, unknown>): boolean => {
    if (op.op === 'method') {
      const args = op['args'] as Expr[]
      return (
        args.some(
          (arg, index) =>
            arg.expr === 'local' &&
            arg.name === name &&
            passedMutably({ expr: 'method', name: op['name'] as string, args }, index),
        ) || args.some(inExpr)
      )
    }
    if (op.op === 'emit') {
      return inExpr(op['event'] as Expr) || (op['at'] !== undefined && inExpr(op['at'] as Expr))
    }
    if (op.op === 'next-change') return inExpr(op['mask'] as Expr)
    if (op.op === 'next-change-before')
      return inExpr(op['mask'] as Expr) || inExpr(op['until'] as Expr)
    return false
  }
  const inStmts = (stmts: Stmt[]): boolean =>
    stmts.some((stmt) => {
      switch (stmt.stmt) {
        case 'assign':
          return rooted(stmt.target) || inExpr(stmt.value)
        case 'let':
          return inExpr(stmt.value)
        case 'let-ref':
          return false
        case 'expr':
          return inExpr(stmt.expr)
        case 'return':
          return stmt.value !== undefined && inExpr(stmt.value)
        case 'if':
          return inExpr(stmt.cond) || inStmts(stmt.then) || inStmts(stmt.else ?? [])
        case 'if-some':
          return inExpr(stmt.value) || inStmts(stmt.then) || inStmts(stmt.else ?? [])
        case 'let-some':
          return inExpr(stmt.value) || inStmts(stmt.else)
        case 'match':
          return inExpr(stmt.value) || stmt.arms.some((arm) => inStmts(arm.body))
        case 'loop':
          return inStmts(stmt.body)
        case 'while':
          return inExpr(stmt.cond) || inStmts(stmt.body)
        case 'for':
          return inExpr(stmt.from) || inExpr(stmt.to) || inStmts(stmt.body)
        case 'for-each':
          return inExpr(stmt.array) || inStmts(stmt.body)
        default:
          return false
      }
    })
  return inStmts(block)
}

function isSelfPlace(expr: Expr): boolean {
  if (expr.expr === 'self') return true
  if (expr.expr === 'field' || expr.expr === 'index') return isSelfPlace(expr.of)
  return false
}

/** A method of an object literal as the arrow it stands for. */
function methodAsArrow(method: ts.MethodDeclaration): ts.Expression {
  if (!method.body) throw new CompileError(method, 'a method has a body')
  return ts.factory.createArrowFunction(
    method.modifiers?.filter((modifier) => modifier.kind === ts.SyntaxKind.AsyncKeyword),
    method.typeParameters,
    method.parameters,
    method.type,
    undefined,
    method.body,
  )
}

/** A literal's value: a number, a string or a boolean. */
function literalValue(node: ts.Expression): boolean | number | string | undefined {
  let bare = node
  while (ts.isParenthesizedExpression(bare)) bare = bare.expression
  if (ts.isNumericLiteral(bare)) return Number(bare.text.replace(/_/g, ''))
  if (ts.isStringLiteral(bare) || ts.isNoSubstitutionTemplateLiteral(bare)) return bare.text
  if (bare.kind === ts.SyntaxKind.TrueKeyword) return true
  if (bare.kind === ts.SyntaxKind.FalseKeyword) return false
  return undefined
}

/** An object literal of literals. */
function literalObject(
  node: ts.ObjectLiteralExpression,
): Record<string, boolean | number | string> {
  const out: Record<string, boolean | number | string> = {}
  for (const property of node.properties) {
    if (!ts.isPropertyAssignment(property) || !ts.isIdentifier(property.name)) {
      throw new CompileError(property, 'a literal object has named properties')
    }
    const value = literalValue(property.initializer)
    if (value === undefined) throw new CompileError(property.initializer, 'a literal is expected')
    out[property.name.text] = value
  }
  return out
}

/** A JSON value from literals: the configuration's defaults. */
function jsonValue(node: ts.Expression): Json | undefined {
  let bare = node
  while (ts.isParenthesizedExpression(bare)) bare = bare.expression
  if (ts.isIdentifier(bare) && bare.text === 'undefined') return null
  const literal = literalValue(bare)
  if (literal !== undefined) return literal
  if (ts.isArrayLiteralExpression(bare)) {
    const items: Json[] = []
    for (const element of bare.elements) {
      const item = jsonValue(element)
      if (item === undefined) return undefined
      items.push(item)
    }
    return items
  }
  if (ts.isObjectLiteralExpression(bare)) {
    const out: { [key: string]: Json } = {}
    for (const property of bare.properties) {
      if (!ts.isPropertyAssignment(property) || !ts.isIdentifier(property.name)) return undefined
      const value = jsonValue(property.initializer)
      if (value === undefined) return undefined
      out[property.name.text] = value
    }
    return out
  }
  return undefined
}
