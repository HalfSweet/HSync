// SPDX-License-Identifier: Apache-2.0

/**
 * Lowering the statements and expressions of a DSL function to the IR: the subset is checked
 * on the way, references to `this` become fields and methods of the decoder, calls to the DSL's
 * helpers become intrinsics and operations, and TypeScript's narrowing of options and tagged
 * unions becomes the IR's binding constructs (`if-some`, `let-some`, `match`, `if-variant`).
 */

import ts from 'typescript'

import type { Arm, BinaryOp, Expr, Literal, Op, Stmt, Type, TypeDef } from './ir.ts'
import { CompileError } from './program.ts'
import type { Types } from './types.ts'

/** What the lowering knows about the decoder a method belongs to. */
export interface DecoderShape {
  kind: 'logic' | 'stream' | 'bus'
  /** The state fields and their types. */
  state: Map<string, Type>
  /** The methods and whether each is async. */
  methods: Map<string, boolean>
}

/** What the lowering knows about the module. */
export interface ModuleShape {
  functions: Set<string>
  consts: Set<string>
  /** Every record, enum and union in scope, by name. */
  types: Map<string, TypeDef>
}

/** The DSL helpers that are intrinsics, by their DSL name. */
const INTRINSICS: Record<string, string> = {
  min: 'min',
  max: 'max',
  saturatingSub: 'saturating-sub',
  countOnes: 'count-ones',
  fromBool: 'from-bool',
  mulDiv: 'mul-div',
  mulDivCeil: 'mul-div-ceil',
  at: 'at',
  one: 'one',
  over: 'over',
  join: 'join',
  mask: 'mask',
  batch: 'batch',
}

const BATCH_PROPERTIES: Record<string, string> = {
  length: 'batch-length',
  capacity: 'batch-capacity',
  isEmpty: 'batch-is-empty',
  isFull: 'batch-is-full',
  at: 'batch-at',
}

const BINARY_OPS: Partial<Record<ts.SyntaxKind, BinaryOp>> = {
  [ts.SyntaxKind.PlusToken]: 'add',
  [ts.SyntaxKind.MinusToken]: 'sub',
  [ts.SyntaxKind.AsteriskToken]: 'mul',
  [ts.SyntaxKind.SlashToken]: 'div',
  [ts.SyntaxKind.PercentToken]: 'rem',
  [ts.SyntaxKind.AmpersandToken]: 'bit-and',
  [ts.SyntaxKind.BarToken]: 'bit-or',
  [ts.SyntaxKind.CaretToken]: 'bit-xor',
  [ts.SyntaxKind.LessThanLessThanToken]: 'shl',
  [ts.SyntaxKind.GreaterThanGreaterThanToken]: 'shr',
  [ts.SyntaxKind.LessThanToken]: 'lt',
  [ts.SyntaxKind.LessThanEqualsToken]: 'le',
  [ts.SyntaxKind.GreaterThanToken]: 'gt',
  [ts.SyntaxKind.GreaterThanEqualsToken]: 'ge',
}

const COMPOUND_OPS: Partial<Record<ts.SyntaxKind, BinaryOp>> = {
  [ts.SyntaxKind.PlusEqualsToken]: 'add',
  [ts.SyntaxKind.MinusEqualsToken]: 'sub',
  [ts.SyntaxKind.AsteriskEqualsToken]: 'mul',
  [ts.SyntaxKind.AmpersandEqualsToken]: 'bit-and',
  [ts.SyntaxKind.BarEqualsToken]: 'bit-or',
  [ts.SyntaxKind.CaretEqualsToken]: 'bit-xor',
  [ts.SyntaxKind.LessThanLessThanEqualsToken]: 'shl',
  [ts.SyntaxKind.GreaterThanGreaterThanEqualsToken]: 'shr',
}

/** How a reference is narrowed in the current scope. */
type Narrowing =
  /** An option known to be present, bound to a local. */
  | { kind: 'some'; bind: string }
  /** A union known to be one variant; its fields are bound to locals as they are read. */
  | { kind: 'variant'; name: string; binds: Map<string, string> }
  /** A local that names a narrowed reference (`const x = this.field` after a check). */
  | { kind: 'alias'; bind: string }

/** A block's scope: the locals it declares, the narrowings in force, and the narrowed places
 * assigned to, which may not be read again in it. */
interface Scope {
  locals: Set<string>
  narrowings: Map<string, Narrowing>
  stale: Set<string>
}

/** What a condition says. */
type Cond =
  | { kind: 'some'; place: ts.Expression; present: boolean }
  | { kind: 'variant'; place: ts.Expression; name: string; is: boolean }
  | { kind: 'bool'; expr: ts.Expression }

/** Lowers the bodies of one module's functions and methods. */
export class Lowerer {
  private scopes: Scope[] = []
  private isMethod = false
  private isAsync = false
  /** Nodes replaced by an expression while an optional chain is lowered. */
  private substitutions = new Map<ts.Node, Expr>()
  /** Optional-chain accesses lowered as plain ones. */
  private plainChain = new Set<ts.Node>()

  readonly types: Types
  readonly module: ModuleShape
  readonly decoder: DecoderShape | undefined

  constructor(types: Types, module: ModuleShape, decoder?: DecoderShape) {
    this.types = types
    this.module = module
    this.decoder = decoder
  }

  /** Lowers a function body with `params` in scope. */
  body(
    params: readonly string[],
    body: ts.ConciseBody,
    options: { method: boolean; async: boolean },
  ): Stmt[] {
    this.scopes = [{ locals: new Set(params), narrowings: new Map(), stale: new Set() }]
    this.isMethod = options.method
    this.isAsync = options.async
    if (ts.isBlock(body)) return this.statements(body.statements)
    return [{ stmt: 'return', value: this.expr(body) }]
  }

  /** Lowers one expression with `params` in scope: a field initializer, a requirement. */
  standalone(params: readonly string[], node: ts.Expression, options: { method: boolean }): Expr {
    this.scopes = [{ locals: new Set(params), narrowings: new Map(), stale: new Set() }]
    this.isMethod = options.method
    this.isAsync = false
    return this.expr(node)
  }

  // Scopes

  private get scope(): Scope {
    const scope = this.scopes[this.scopes.length - 1]
    if (!scope) throw new Error('no scope')
    return scope
  }

  private addLocal(name: string) {
    this.scope.locals.add(name)
  }

  private declared(name: string): boolean {
    return this.scopes.some((scope) => scope.locals.has(name))
  }

  private fresh(candidate: string): string {
    if (!this.declared(candidate)) return candidate
    for (let n = 2; ; n++) {
      const name = `${candidate}_${n}`
      if (!this.declared(name)) return name
    }
  }

  private inScope<T>(f: () => T): T {
    this.scopes.push({ locals: new Set(), narrowings: new Map(), stale: new Set() })
    try {
      return f()
    } finally {
      this.scopes.pop()
    }
  }

  private narrowing(key: string): Narrowing | undefined {
    for (let i = this.scopes.length - 1; i >= 0; i--) {
      const scope = this.scopes[i]
      if (scope?.stale.has(key)) return undefined
      const found = scope?.narrowings.get(key)
      if (found) return found
    }
    return undefined
  }

  private isStale(key: string): boolean {
    return this.scopes.some((scope) => scope.stale.has(key))
  }

  /** A canonical name for a place: `self.transfer`, `local.config.cs`. */
  private key(node: ts.Expression): string | undefined {
    const bare = unparen(node)
    if (bare.kind === ts.SyntaxKind.ThisKeyword) return 'self'
    if (ts.isIdentifier(bare)) return `local.${bare.text}`
    if (ts.isPropertyAccessExpression(bare)) {
      const base = this.key(bare.expression)
      return base === undefined ? undefined : `${base}.${bare.name.text}`
    }
    return undefined
  }

  /** The last name of a place, for naming what it is bound to. */
  private leaf(node: ts.Expression): string {
    const bare = unparen(node)
    if (ts.isIdentifier(bare)) return bare.text
    if (ts.isPropertyAccessExpression(bare)) return bare.name.text
    return 'value'
  }

  // Statements

  private statements(nodes: readonly ts.Statement[]): Stmt[] {
    return this.inScope(() => {
      const out: Stmt[] = []
      for (const node of nodes) out.push(...this.statement(node))
      return out
    })
  }

  /** A statement as a block of its own: the body of a loop or a branch. */
  private block(node: ts.Statement | undefined): Stmt[] {
    if (!node) return []
    if (ts.isBlock(node)) return this.statements(node.statements)
    return this.statements([node])
  }

  private statement(node: ts.Statement): Stmt[] {
    if (ts.isVariableStatement(node)) return this.variables(node.declarationList)
    if (ts.isExpressionStatement(node)) return [this.expressionStatement(node.expression)]
    if (ts.isIfStatement(node)) {
      return this.ifStatement(conjuncts(node.expression), node.thenStatement, node.elseStatement)
    }
    if (ts.isSwitchStatement(node)) return [this.switchStatement(node)]
    if (ts.isForStatement(node)) return [this.forStatement(node)]
    if (ts.isForOfStatement(node)) return [this.forOf(node)]
    if (ts.isWhileStatement(node)) {
      return [{ stmt: 'while', cond: this.expr(node.expression), body: this.block(node.statement) }]
    }
    if (ts.isLabeledStatement(node)) {
      const [inner, ...rest] = this.statement(node.statement)
      if (rest.length || !inner || !('label' in inner)) {
        throw new CompileError(node, 'a label names a loop')
      }
      return [{ ...inner, label: node.label.text }]
    }
    if (ts.isBreakStatement(node)) return [{ stmt: 'break', ...label(node.label) }]
    if (ts.isContinueStatement(node)) return [{ stmt: 'continue', ...label(node.label) }]
    if (ts.isReturnStatement(node)) {
      return [
        node.expression
          ? { stmt: 'return', value: this.expr(node.expression) }
          : { stmt: 'return' },
      ]
    }
    if (ts.isBlock(node)) return this.statements(node.statements)
    if (ts.isEmptyStatement(node)) return []
    throw new CompileError(node, `${ts.SyntaxKind[node.kind]} is not in the subset`)
  }

  private variables(list: ts.VariableDeclarationList): Stmt[] {
    const out: Stmt[] = []
    for (const decl of list.declarations) {
      if (!ts.isIdentifier(decl.name)) {
        throw new CompileError(decl, 'destructuring is not in the subset')
      }
      if (!decl.initializer)
        throw new CompileError(decl, 'a local is initialized where it is declared')
      const name = decl.name.text
      const init = unparen(decl.initializer)
      // `const x = this.field` after a check of that field: `x` names what the check bound.
      const key = this.key(init)
      const narrowed = key === undefined ? undefined : this.narrowing(key)
      if (narrowed && (narrowed.kind === 'some' || narrowed.kind === 'alias')) {
        if (list.flags & ts.NodeFlags.Let) {
          throw new CompileError(decl, 'a narrowed reference is named with const')
        }
        this.scope.narrowings.set(`local.${name}`, { kind: 'alias', bind: narrowed.bind })
        this.addLocal(name)
        continue
      }
      const stmt: Stmt = { stmt: 'let', name, value: this.expr(init) }
      if (list.flags & ts.NodeFlags.Let) stmt.mutable = true
      const type = decl.type ? this.types.mapNode(decl.type) : this.inferred(init)
      if (type !== undefined) stmt.type = type
      if (!decl.type && isLiteral(init) && !isBoolean(init)) {
        throw new CompileError(decl, 'an integer local declared with a literal says its width')
      }
      this.addLocal(name)
      out.push(stmt)
    }
    return out
  }

  /** The type of a local from its initializer, when it has an IR type. */
  private inferred(init: ts.Expression): Type | undefined {
    const type = isReference(init)
      ? this.types.declaredType(init)
      : this.types.checker.getTypeAtLocation(init)
    const mapped = this.types.map(type)
    if (typeof mapped === 'object' && 'batch' in mapped) {
      throw new CompileError(init, 'a batch is a field of the decoder, not a local')
    }
    return mapped
  }

  private expressionStatement(node: ts.Expression): Stmt {
    const bare = unparen(node)
    if (ts.isBinaryExpression(bare)) {
      const op = bare.operatorToken.kind
      if (op === ts.SyntaxKind.EqualsToken) {
        const value = this.expr(bare.right)
        return { stmt: 'assign', target: this.target(bare.left), value }
      }
      const compound = COMPOUND_OPS[op]
      if (compound) {
        // A narrowed option is written as a whole: its new value is computed from the binding.
        const key = this.key(bare.left)
        if (key !== undefined && this.narrowing(key)?.kind === 'some') {
          const current = this.expr(bare.left)
          const right = this.expr(bare.right)
          const value: Expr = { expr: 'binary', op: compound, left: current, right }
          return { stmt: 'assign', target: this.target(bare.left), value }
        }
        const value = this.expr(bare.right)
        return { stmt: 'assign', target: this.target(bare.left), op: compound, value }
      }
    }
    if (ts.isPostfixUnaryExpression(bare) || ts.isPrefixUnaryExpression(bare)) {
      const step = incrementOf(bare)
      if (step) {
        return {
          stmt: 'assign',
          target: this.target(step.target),
          op: step.op,
          value: { expr: 'lit', value: 1 },
        }
      }
    }
    return { stmt: 'expr', expr: this.expr(bare) }
  }

  /** The place an assignment writes to. The whole of a narrowed option is written as the
   * option itself, and may not be read again in the block; a narrowed prefix is written
   * through its binding. */
  private target(node: ts.Expression): Expr {
    const bare = unparen(node)
    const key = this.key(bare)
    const narrowed = key === undefined ? undefined : this.narrowing(key)
    if (narrowed?.kind === 'alias') return { expr: 'local', name: narrowed.bind }
    if (narrowed?.kind === 'some' && key !== undefined) {
      this.scope.stale.add(key)
      return this.place(bare)
    }
    if (narrowed?.kind === 'variant') {
      throw new CompileError(node, 'a union known to be one variant is not reassigned as a whole')
    }
    const lowered = this.expr(bare)
    if (!isPlace(lowered))
      throw new CompileError(node, 'the target of an assignment is a local, a field or an item')
    return lowered
  }

  /** A place lowered without the narrowing of its last step. */
  private place(node: ts.Expression): Expr {
    const bare = unparen(node)
    if (ts.isIdentifier(bare)) {
      if (!this.declared(bare.text)) throw new CompileError(bare, `${bare.text} is not in scope`)
      return { expr: 'local', name: bare.text }
    }
    if (ts.isPropertyAccessExpression(bare)) {
      if (bare.expression.kind === ts.SyntaxKind.ThisKeyword) return this.selfField(bare)
      return { expr: 'field', of: this.expr(bare.expression), name: bare.name.text }
    }
    throw new CompileError(node, 'not a place')
  }

  // Conditions

  /** What a condition says: a test of an option, of a union's kind, or anything else. */
  private split(node: ts.Expression): Cond {
    const bare = unparen(node)
    if (ts.isBinaryExpression(bare)) {
      const op = bare.operatorToken.kind
      const equal = op === ts.SyntaxKind.EqualsEqualsEqualsToken
      const unequal = op === ts.SyntaxKind.ExclamationEqualsEqualsToken
      if (equal || unequal) {
        const left = unparen(bare.left)
        const right = unparen(bare.right)
        if (isUndefined(right) && this.key(left) !== undefined) {
          return { kind: 'some', place: left, present: unequal }
        }
        if (isUndefined(left) && this.key(right) !== undefined) {
          return { kind: 'some', place: right, present: unequal }
        }
        if (
          ts.isPropertyAccessExpression(left) &&
          left.name.text === 'kind' &&
          ts.isStringLiteral(right) &&
          this.types.unionOf(this.types.declaredType(left.expression)) !== undefined
        ) {
          return { kind: 'variant', place: left.expression, name: right.text, is: equal }
        }
      }
    }
    return { kind: 'bool', expr: bare }
  }

  /** Lowers `if` over the conjuncts of its condition, nesting a binding construct for every
   * conjunct that narrows. The else branch is repeated where the nesting needs it. */
  private ifStatement(
    conds: ts.Expression[],
    thenStmt: ts.Statement,
    elseStmt: ts.Statement | undefined,
  ): Stmt[] {
    const [first, ...rest] = conds
    if (!first) return this.block(thenStmt)
    const cond = this.split(first)
    // The branches are blocks of their own: what a nested test binds stays in them.
    const nested = () => this.inScope(() => this.ifStatement(rest, thenStmt, elseStmt))
    const inner = () => (rest.length ? nested() : this.block(thenStmt))
    if (cond.kind === 'some') {
      const value = this.expr(cond.place)
      const bind = this.bindName(cond.place)
      if (cond.present) {
        const then = this.narrowed(cond.place, { kind: 'some', bind }, bind, inner)
        const stmt: Stmt = { stmt: 'if-some', value, bind, then }
        const otherwise = this.block(elseStmt)
        if (otherwise.length) stmt.else = otherwise
        return [stmt]
      }
      // `if (x === undefined) leave` binds x for the rest of the block.
      if (!rest.length && !elseStmt && leaves(thenStmt)) {
        const otherwise = this.block(thenStmt)
        const key = this.key(cond.place)
        if (key !== undefined) this.scope.narrowings.set(key, { kind: 'some', bind })
        this.addLocal(bind)
        return [{ stmt: 'let-some', bind, value, else: otherwise }]
      }
      const then = this.narrowed(cond.place, { kind: 'some', bind }, bind, () =>
        this.block(elseStmt),
      )
      const otherwise = rest.length ? nested() : this.block(thenStmt)
      const stmt: Stmt = { stmt: 'if-some', value, bind, then }
      if (otherwise.length) stmt.else = otherwise
      return [stmt]
    }
    if (cond.kind === 'variant') {
      const value = this.expr(cond.place)
      const binds = new Map<string, string>()
      const narrowing: Narrowing = { kind: 'variant', name: cond.name, binds }
      const matched = cond.is
        ? this.narrowed(cond.place, narrowing, undefined, inner)
        : this.narrowed(cond.place, narrowing, undefined, () => this.block(elseStmt))
      const other = cond.is ? this.block(elseStmt) : rest.length ? nested() : this.block(thenStmt)
      const arm: Arm = { pattern: { pattern: 'variant', name: cond.name }, body: matched }
      if (binds.size)
        arm.pattern = { pattern: 'variant', name: cond.name, binds: Object.fromEntries(binds) }
      return [
        { stmt: 'match', value, arms: [arm, { pattern: { pattern: 'default' }, body: other }] },
      ]
    }
    // Plain conditions in a row make one test.
    const plain = [first]
    while (rest.length && this.split(rest[0] as ts.Expression).kind === 'bool') {
      plain.push(rest.shift() as ts.Expression)
    }
    const test = plain
      .map((expression) => this.expr(expression))
      .reduce((left, right) => ({ expr: 'binary', op: 'and', left, right }))
    const stmt: Stmt = { stmt: 'if', cond: test, then: inner() }
    const otherwise = this.block(elseStmt)
    if (otherwise.length) stmt.else = otherwise
    return [stmt]
  }

  /** The name an option's value is bound to: the place's own name, unless taken — a local
   * narrowed gets a new name, so that the IR tells its value from the local itself. */
  private bindName(place: ts.Expression): string {
    return this.fresh(this.leaf(unparen(place)))
  }

  /** Runs `f` in a scope where `place` is narrowed, with `declare` (the bound local, if
   * any) in scope. */
  private narrowed<T>(
    place: ts.Expression,
    narrowing: Narrowing,
    declare: string | undefined,
    f: () => T,
  ): T {
    return this.inScope(() => {
      const key = this.key(place)
      if (key === undefined) throw new CompileError(place, 'only a local or a field is narrowed')
      this.scope.narrowings.set(key, narrowing)
      if (declare !== undefined) this.addLocal(declare)
      return f()
    })
  }

  // Switch

  private switchStatement(node: ts.SwitchStatement): Stmt {
    const subject = unparen(node.expression)
    const onKind =
      ts.isPropertyAccessExpression(subject) &&
      subject.name.text === 'kind' &&
      this.types.unionOf(this.types.declaredType(subject.expression)) !== undefined
        ? subject.expression
        : undefined
    const value = this.expr(onKind ?? subject)
    const arms: Arm[] = []
    let pending: Literal[] = []
    let pendingDefault = false
    for (const clause of node.caseBlock.clauses) {
      if (ts.isDefaultClause(clause)) pendingDefault = true
      else pending.push(this.caseValue(clause.expression))
      if (!clause.statements.length) continue
      const statements =
        clause.statements.length === 1 && ts.isBlock(clause.statements[0] as ts.Node)
          ? (clause.statements[0] as ts.Block).statements
          : clause.statements
      const body = [...statements]
      const last = body[body.length - 1]
      if (last && ts.isBreakStatement(last) && !last.label) body.pop()
      for (const stmt of body) {
        if (ts.isBreakStatement(stmt)) {
          throw new CompileError(stmt, 'a case leaves by returning, or by its last statement')
        }
      }
      if (onKind) {
        for (const name of pending) {
          const binds = new Map<string, string>()
          const lowered = this.narrowed(
            onKind,
            { kind: 'variant', name: String(name), binds },
            undefined,
            () => this.statements(body),
          )
          const arm: Arm = { pattern: { pattern: 'variant', name: String(name) }, body: lowered }
          if (binds.size)
            arm.pattern = {
              pattern: 'variant',
              name: String(name),
              binds: Object.fromEntries(binds),
            }
          arms.push(arm)
        }
      } else if (pending.length) {
        arms.push({ pattern: { pattern: 'values', values: pending }, body: this.statements(body) })
      }
      if (pendingDefault) {
        arms.push({ pattern: { pattern: 'default' }, body: this.statements(body) })
      }
      pending = []
      pendingDefault = false
    }
    if (pending.length || pendingDefault) {
      throw new CompileError(node, 'the last case of a switch has a body')
    }
    return { stmt: 'match', value, arms }
  }

  private caseValue(node: ts.Expression): Literal {
    const bare = unparen(node)
    if (ts.isStringLiteral(bare)) return bare.text
    if (ts.isNumericLiteral(bare)) return numeric(bare)
    if (bare.kind === ts.SyntaxKind.TrueKeyword) return true
    if (bare.kind === ts.SyntaxKind.FalseKeyword) return false
    throw new CompileError(node, 'a case is a literal')
  }

  // Loops

  private forStatement(node: ts.ForStatement): Stmt {
    if (!node.initializer && !node.condition && !node.incrementor) {
      return { stmt: 'loop', body: this.block(node.statement) }
    }
    const init = node.initializer
    if (
      !init ||
      !ts.isVariableDeclarationList(init) ||
      init.declarations.length !== 1 ||
      !node.condition ||
      !node.incrementor
    ) {
      throw new CompileError(node, 'a counted loop is `for (let i = a; i < b; i++)`')
    }
    const decl = init.declarations[0] as ts.VariableDeclaration
    if (!ts.isIdentifier(decl.name) || !decl.initializer) {
      throw new CompileError(decl, 'a counted loop declares its counter')
    }
    const counter = decl.name.text
    const cond = unparen(node.condition)
    if (
      !ts.isBinaryExpression(cond) ||
      !ts.isIdentifier(unparen(cond.left)) ||
      (unparen(cond.left) as ts.Identifier).text !== counter ||
      (cond.operatorToken.kind !== ts.SyntaxKind.LessThanToken &&
        cond.operatorToken.kind !== ts.SyntaxKind.LessThanEqualsToken)
    ) {
      throw new CompileError(node.condition, 'a counted loop runs while `i < b` or `i <= b`')
    }
    const step = incrementOf(node.incrementor)
    if (!step || step.op !== 'add' || this.leaf(step.target) !== counter) {
      throw new CompileError(node.incrementor, 'a counted loop steps by `i++`')
    }
    const from = this.expr(decl.initializer)
    let to = this.expr(cond.right)
    if (cond.operatorToken.kind === ts.SyntaxKind.LessThanEqualsToken) {
      to = { expr: 'binary', op: 'add', left: to, right: { expr: 'lit', value: 1 } }
    }
    return this.inScope(() => {
      this.addLocal(counter)
      return { stmt: 'for', var: counter, from, to, body: this.block(node.statement) }
    })
  }

  private forOf(node: ts.ForOfStatement): Stmt {
    const init = node.initializer
    if (!ts.isVariableDeclarationList(init) || init.declarations.length !== 1) {
      throw new CompileError(node, 'for…of declares its item')
    }
    const decl = init.declarations[0] as ts.VariableDeclaration
    if (!ts.isIdentifier(decl.name)) throw new CompileError(decl, 'for…of names its item')
    const array = this.expr(node.expression)
    return this.inScope(() => {
      this.addLocal(decl.name.getText())
      return { stmt: 'for-each', var: decl.name.getText(), array, body: this.block(node.statement) }
    })
  }

  // Expressions

  expr(node: ts.Expression): Expr {
    const substituted = this.substitutions.get(node)
    if (substituted) return substituted
    const bare = unparen(node)
    if (bare !== node) return this.expr(bare)
    if (ts.isNumericLiteral(bare)) return { expr: 'lit', value: numeric(bare) }
    if (bare.kind === ts.SyntaxKind.TrueKeyword) return { expr: 'lit', value: true }
    if (bare.kind === ts.SyntaxKind.FalseKeyword) return { expr: 'lit', value: false }
    if (ts.isStringLiteral(bare) || ts.isNoSubstitutionTemplateLiteral(bare)) {
      return this.string(bare, this.types.checker.getContextualType(bare))
    }
    if (ts.isTemplateExpression(bare)) return this.template(bare)
    if (ts.isIdentifier(bare)) return this.identifier(bare)
    if (bare.kind === ts.SyntaxKind.ThisKeyword) {
      throw new CompileError(bare, 'this is read through its fields and methods')
    }
    if (ts.isPropertyAccessExpression(bare)) return this.access(bare)
    if (ts.isElementAccessExpression(bare)) {
      return {
        expr: 'index',
        of: this.expr(bare.expression),
        at: this.expr(bare.argumentExpression),
      }
    }
    if (ts.isNonNullExpression(bare)) return { expr: 'required', value: this.expr(bare.expression) }
    if (ts.isCallExpression(bare)) return this.call(bare, false)
    if (ts.isAwaitExpression(bare)) {
      const operand = unparen(bare.expression)
      if (!ts.isCallExpression(operand)) throw new CompileError(bare, 'await takes an operation')
      return this.call(operand, true)
    }
    if (ts.isPrefixUnaryExpression(bare)) {
      if (bare.operator === ts.SyntaxKind.ExclamationToken) {
        return { expr: 'unary', op: 'not', value: this.expr(bare.operand) }
      }
      if (bare.operator === ts.SyntaxKind.TildeToken) {
        return { expr: 'unary', op: 'bit-not', value: this.expr(bare.operand) }
      }
      throw new CompileError(bare, 'integers are unsigned: no unary minus or plus')
    }
    if (ts.isBinaryExpression(bare)) return this.binary(bare)
    if (ts.isConditionalExpression(bare)) return this.conditional(bare)
    if (ts.isObjectLiteralExpression(bare)) return this.object(bare)
    if (ts.isArrayLiteralExpression(bare)) {
      return { expr: 'array', items: bare.elements.map((element) => this.expr(element)) }
    }
    if (ts.isAsExpression(bare)) {
      return { expr: 'cast', value: this.expr(bare.expression), to: this.types.mapNode(bare.type) }
    }
    throw new CompileError(bare, `${ts.SyntaxKind[bare.kind]} is not in the subset`)
  }

  private string(node: ts.StringLiteralLike, type: ts.Type | undefined): Expr {
    const enumName = type === undefined ? undefined : this.types.enumOf(type)
    if (enumName !== undefined) return { expr: 'enum', type: enumName, value: node.text }
    return { expr: 'text', value: node.text }
  }

  private template(node: ts.TemplateExpression): Expr {
    const parts: Expr[] = []
    if (node.head.text) parts.push({ expr: 'text', value: node.head.text })
    for (const span of node.templateSpans) {
      parts.push(this.expr(span.expression))
      if (span.literal.text) parts.push({ expr: 'text', value: span.literal.text })
    }
    return { expr: 'format', parts }
  }

  private identifier(node: ts.Identifier): Expr {
    if (node.text === 'undefined') return { expr: 'none' }
    const key = `local.${node.text}`
    if (this.isStale(key)) {
      throw new CompileError(
        node,
        `${node.text} was assigned after it was narrowed; read it in a new check`,
      )
    }
    const narrowed = this.narrowing(key)
    if (narrowed?.kind === 'some' || narrowed?.kind === 'alias') {
      return { expr: 'local', name: narrowed.bind }
    }
    if (this.declared(node.text)) return { expr: 'local', name: node.text }
    if (this.module.consts.has(node.text)) return { expr: 'local', name: node.text }
    if (this.types.libName(node) === 'MAX_CHANNELS') {
      return { expr: 'intrinsic', name: 'max-channels', args: [] }
    }
    throw new CompileError(node, `${node.text} is not in scope`)
  }

  private access(node: ts.PropertyAccessExpression): Expr {
    if (node.flags & ts.NodeFlags.OptionalChain && !this.plainChain.has(node)) {
      return this.optionalChain(node)
    }
    const key = this.key(node)
    if (key !== undefined) {
      if (this.isStale(key)) {
        throw new CompileError(
          node,
          `${node.getText()} was assigned after it was narrowed; read it in a new check`,
        )
      }
      const narrowed = this.narrowing(key)
      if (narrowed?.kind === 'some' || narrowed?.kind === 'alias') {
        return { expr: 'local', name: narrowed.bind }
      }
    }
    const receiver = unparen(node.expression)
    const name = node.name.text
    const receiverKey = this.key(receiver)
    if (receiverKey !== undefined) {
      const narrowed = this.narrowing(receiverKey)
      if (narrowed?.kind === 'variant') {
        if (name === 'kind') throw new CompileError(node, 'the kind is known here')
        let bind = narrowed.binds.get(name)
        if (bind === undefined) {
          bind = this.fresh(name)
          if (bind !== name) bind = this.fresh(`${this.leaf(receiver)}_${name}`)
          narrowed.binds.set(name, bind)
          this.addLocal(bind)
        }
        return { expr: 'local', name: bind }
      }
    }
    if (receiver.kind === ts.SyntaxKind.ThisKeyword) return this.selfField(node)
    if (
      ts.isPropertyAccessExpression(receiver) &&
      receiver.expression.kind === ts.SyntaxKind.ThisKeyword &&
      receiver.name.text === 'signal'
    ) {
      if (name === 'position') return { expr: 'intrinsic', name: 'position', args: [] }
      throw new CompileError(node, 'the signal cursor has operations, not fields')
    }
    const receiverType = this.types.declaredType(receiver)
    const lib = this.types.isLib(receiverType.symbol) ? receiverType.symbol.name : undefined
    if (lib === 'Batch') {
      const intrinsic = BATCH_PROPERTIES[name]
      if (!intrinsic) throw new CompileError(node, `a batch has no ${name}`)
      return { expr: 'intrinsic', name: intrinsic, args: [this.expr(receiver)] }
    }
    if (lib === 'Transaction') {
      if (name === 'id') return { expr: 'intrinsic', name: 'tx-id', args: [this.expr(receiver)] }
      if (name === 'partial')
        return { expr: 'intrinsic', name: 'tx-partial', args: [this.expr(receiver)] }
      throw new CompileError(node, `a transaction has no ${name}`)
    }
    if (name === 'kind')
      throw new CompileError(node, 'the kind of a union is read in a switch or an if')
    return { expr: 'field', of: this.expr(receiver), name }
  }

  /** `a?.b.c`: `c` of `b` when `a` is present. */
  private optionalChain(top: ts.PropertyAccessExpression): Expr {
    let inner: ts.Expression = top
    const chain: ts.Node[] = []
    while (
      ts.isPropertyAccessExpression(inner) &&
      inner.flags & ts.NodeFlags.OptionalChain &&
      !inner.questionDotToken
    ) {
      chain.push(inner)
      inner = inner.expression
    }
    if (!ts.isPropertyAccessExpression(inner) || !inner.questionDotToken) {
      throw new CompileError(top, 'only `a?.b` chains are in the subset')
    }
    chain.push(inner)
    const base = inner.expression
    const value = this.expr(base)
    const bind = this.fresh(this.leaf(base))
    const some = this.inScope(() => {
      this.addLocal(bind)
      this.substitutions.set(base, { expr: 'local', name: bind })
      for (const node of chain) this.plainChain.add(node)
      try {
        return this.expr(top)
      } finally {
        this.substitutions.delete(base)
        for (const node of chain) this.plainChain.delete(node)
      }
    })
    return { expr: 'if-some', value, bind, some, none: { expr: 'none' } }
  }

  private selfField(node: ts.PropertyAccessExpression): Expr {
    const name = node.name.text
    if (!this.isMethod) throw new CompileError(node, 'this is only in scope in the decoder')
    if (name === 'config' || name === 'rate' || this.decoder?.state.has(name)) {
      return { expr: 'self', field: name }
    }
    if (name === 'signal' || name === 'out' || name === 'input') {
      throw new CompileError(node, `this.${name} is used through its operations`)
    }
    throw new CompileError(node, `the decoder has no field ${name}`)
  }

  private call(node: ts.CallExpression, awaited: boolean): Expr {
    const callee = unparen(node.expression)
    const args = () => node.arguments.map((arg) => this.expr(arg))
    const operation = (op: Op): Expr => {
      if (!awaited) throw new CompileError(node, 'an operation is awaited')
      if (!this.isAsync) throw new CompileError(node, 'only an async method awaits')
      return { expr: 'await', op }
    }
    const plain = (expr: Expr): Expr => {
      if (awaited) throw new CompileError(node, 'only an operation or an async method is awaited')
      return expr
    }
    if (ts.isIdentifier(callee)) {
      const lib = this.types.libName(callee)
      if (lib !== undefined) {
        const intrinsic = INTRINSICS[lib]
        if (!intrinsic) throw new CompileError(callee, `${lib} is not called in a decoder`)
        return plain({ expr: 'intrinsic', name: intrinsic, args: args() })
      }
      if (this.module.functions.has(callee.text)) {
        return plain({ expr: 'call', name: callee.text, args: args() })
      }
      throw new CompileError(callee, `${callee.text} is not a function of this module`)
    }
    if (!ts.isPropertyAccessExpression(callee)) {
      throw new CompileError(node, 'a call names a function or a method')
    }
    const receiver = unparen(callee.expression)
    const name = callee.name.text
    if (receiver.kind === ts.SyntaxKind.ThisKeyword) {
      if (!this.isMethod) throw new CompileError(node, 'this is only in scope in the decoder')
      if (name === 'consumed') return plain({ expr: 'intrinsic', name: 'consumed', args: [] })
      const isAsync = this.decoder?.methods.get(name)
      if (isAsync === undefined) throw new CompileError(callee, `the decoder has no method ${name}`)
      if (isAsync) return operation({ op: 'method', name, args: args() })
      return plain({ expr: 'method', name, args: args() })
    }
    if (
      ts.isPropertyAccessExpression(receiver) &&
      receiver.expression.kind === ts.SyntaxKind.ThisKeyword
    ) {
      const handle = receiver.name.text
      if (handle === 'signal') {
        if (name === 'nextChange')
          return operation({ op: 'next-change', mask: this.argument(node, 0) })
        if (name === 'nextChangeBefore') {
          return operation({
            op: 'next-change-before',
            mask: this.argument(node, 0),
            until: this.argument(node, 1),
          })
        }
        if (name === 'sample') return operation({ op: 'sample' })
        if (name === 'seek') return plain({ expr: 'intrinsic', name: 'seek', args: args() })
        throw new CompileError(callee, `the signal cursor has no ${name}`)
      }
      if (handle === 'out') {
        if (name !== 'emit') throw new CompileError(callee, `the outbox has no ${name}`)
        const op: Op = { op: 'emit', event: this.argument(node, 0) }
        if (node.arguments.length > 1) op.at = this.argument(node, 1)
        return operation(op)
      }
      if (handle === 'input') {
        if (name !== 'next') throw new CompileError(callee, `the stream has no ${name}`)
        return operation({ op: 'next' })
      }
    }
    const receiverType = this.types.declaredType(receiver)
    const lib = this.types.isLib(receiverType.symbol) ? receiverType.symbol.name : undefined
    switch (lib) {
      case 'Transaction':
        if (name === 'item') return operation({ op: 'item' })
        if (name === 'itemOrEnd') return operation({ op: 'item-or-end' })
        if (name === 'drain') return operation({ op: 'drain' })
        break
      case 'Levels':
        if (name === 'high') {
          return plain({
            expr: 'intrinsic',
            name: 'levels-high',
            args: [this.expr(receiver), ...args()],
          })
        }
        break
      case 'ChannelMask':
        if (name === 'has' || name === 'with') {
          return plain({
            expr: 'intrinsic',
            name: `mask-${name}`,
            args: [this.expr(receiver), ...args()],
          })
        }
        break
      case 'Batch':
        if (name === 'push' || name === 'take' || name === 'clear') {
          return plain({
            expr: 'intrinsic',
            name: `batch-${name}`,
            args: [this.expr(receiver), ...args()],
          })
        }
        break
      default:
        break
    }
    throw new CompileError(callee, `${name} is not an operation of the subset`)
  }

  private argument(node: ts.CallExpression, index: number): Expr {
    const arg = node.arguments[index]
    if (!arg) throw new CompileError(node, `argument ${index + 1} is missing`)
    return this.expr(arg)
  }

  private binary(node: ts.BinaryExpression): Expr {
    const op = node.operatorToken.kind
    if (op === ts.SyntaxKind.QuestionQuestionToken) {
      return {
        expr: 'intrinsic',
        name: 'unwrap-or',
        args: [this.expr(node.left), this.expr(node.right)],
      }
    }
    if (op === ts.SyntaxKind.AmpersandAmpersandToken) {
      return this.and(conjuncts(node))
    }
    if (op === ts.SyntaxKind.BarBarToken) {
      return this.or(disjuncts(node))
    }
    if (
      op === ts.SyntaxKind.EqualsEqualsEqualsToken ||
      op === ts.SyntaxKind.ExclamationEqualsEqualsToken
    ) {
      return this.equality(node, op === ts.SyntaxKind.EqualsEqualsEqualsToken)
    }
    const mapped = BINARY_OPS[op]
    if (!mapped) {
      throw new CompileError(node.operatorToken, `${ts.tokenToString(op)} is not in the subset`)
    }
    return { expr: 'binary', op: mapped, left: this.expr(node.left), right: this.expr(node.right) }
  }

  /** `a && b && c` as an expression, narrowing where a conjunct does. */
  private and(list: ts.Expression[]): Expr {
    const [first, ...rest] = list
    if (!first) return { expr: 'lit', value: true }
    const cond = this.split(first)
    const tail = (): Expr => (rest.length ? this.and(rest) : { expr: 'lit', value: true })
    if (cond.kind === 'some' && cond.present && rest.length) {
      const value = this.expr(cond.place)
      const bind = this.bindName(cond.place)
      const some = this.narrowed(cond.place, { kind: 'some', bind }, bind, tail)
      return { expr: 'if-some', value, bind, some, none: { expr: 'lit', value: false } }
    }
    if (cond.kind === 'variant' && cond.is && rest.length) {
      const value = this.expr(cond.place)
      const binds = new Map<string, string>()
      const then = this.narrowed(
        cond.place,
        { kind: 'variant', name: cond.name, binds },
        undefined,
        tail,
      )
      const expr: Expr = {
        expr: 'if-variant',
        value,
        variant: cond.name,
        then,
        else: { expr: 'lit', value: false },
      }
      if (binds.size) expr.binds = Object.fromEntries(binds)
      return expr
    }
    const left = this.test(cond)
    if (!rest.length) return left
    return { expr: 'binary', op: 'and', left, right: this.and(rest) }
  }

  /** `a || b || c` as an expression, narrowing where a disjunct's failure does. */
  private or(list: ts.Expression[]): Expr {
    const [first, ...rest] = list
    if (!first) return { expr: 'lit', value: false }
    const cond = this.split(first)
    const tail = (): Expr => (rest.length ? this.or(rest) : { expr: 'lit', value: false })
    if (cond.kind === 'some' && !cond.present && rest.length) {
      const value = this.expr(cond.place)
      const bind = this.bindName(cond.place)
      const some = this.narrowed(cond.place, { kind: 'some', bind }, bind, tail)
      return { expr: 'if-some', value, bind, some, none: { expr: 'lit', value: true } }
    }
    if (cond.kind === 'variant' && !cond.is && rest.length) {
      const value = this.expr(cond.place)
      const binds = new Map<string, string>()
      const then = this.narrowed(
        cond.place,
        { kind: 'variant', name: cond.name, binds },
        undefined,
        tail,
      )
      const expr: Expr = {
        expr: 'if-variant',
        value,
        variant: cond.name,
        then,
        else: { expr: 'lit', value: true },
      }
      if (binds.size) expr.binds = Object.fromEntries(binds)
      return expr
    }
    const left = this.test(cond)
    if (!rest.length) return left
    return { expr: 'binary', op: 'or', left, right: this.or(rest) }
  }

  /** A condition as a boolean value. */
  private test(cond: Cond): Expr {
    if (cond.kind === 'bool') return this.expr(cond.expr)
    if (cond.kind === 'some') {
      return {
        expr: 'if-some',
        value: this.expr(cond.place),
        bind: this.fresh('_present'),
        some: { expr: 'lit', value: cond.present },
        none: { expr: 'lit', value: !cond.present },
      }
    }
    return {
      expr: 'if-variant',
      value: this.expr(cond.place),
      variant: cond.name,
      then: { expr: 'lit', value: cond.is },
      else: { expr: 'lit', value: !cond.is },
    }
  }

  private equality(node: ts.BinaryExpression, equal: boolean): Expr {
    const cond = this.split(node)
    if (cond.kind !== 'bool') return this.test(cond)
    const left = unparen(node.left)
    const right = unparen(node.right)
    const op: BinaryOp = equal ? 'eq' : 'ne'
    // A string literal is an enum value, of the other side's enum.
    if (ts.isStringLiteral(right)) {
      return {
        expr: 'binary',
        op,
        left: this.expr(left),
        right: this.string(right, this.types.declaredType(left)),
      }
    }
    if (ts.isStringLiteral(left)) {
      return {
        expr: 'binary',
        op,
        left: this.string(left, this.types.declaredType(right)),
        right: this.expr(right),
      }
    }
    // An option compared with a value is equal only when present.
    const optional = (side: ts.Expression) => {
      const key = this.key(side)
      if (key !== undefined && this.narrowing(key) !== undefined) return false
      const type = this.types.declaredType(side)
      return this.types.checker.getNonNullableType(type) !== type
    }
    if (optional(left) && !optional(right)) return this.optionEquality(left, right, equal)
    if (optional(right) && !optional(left)) return this.optionEquality(right, left, equal)
    return { expr: 'binary', op, left: this.expr(left), right: this.expr(right) }
  }

  private optionEquality(option: ts.Expression, other: ts.Expression, equal: boolean): Expr {
    const value = this.expr(option)
    const bind = this.fresh(this.leaf(option))
    const some = this.inScope<Expr>(() => {
      this.addLocal(bind)
      return {
        expr: 'binary',
        op: equal ? 'eq' : 'ne',
        left: { expr: 'local', name: bind },
        right: this.expr(other),
      }
    })
    return { expr: 'if-some', value, bind, some, none: { expr: 'lit', value: !equal } }
  }

  private conditional(node: ts.ConditionalExpression): Expr {
    const list = conjuncts(node.condition)
    return this.conditionalOver(list, node.whenTrue, node.whenFalse)
  }

  private conditionalOver(
    list: ts.Expression[],
    whenTrue: ts.Expression,
    whenFalse: ts.Expression,
  ): Expr {
    const [first, ...rest] = list
    if (!first) return this.expr(whenTrue)
    const cond = this.split(first)
    const inner = () =>
      rest.length ? this.conditionalOver(rest, whenTrue, whenFalse) : this.expr(whenTrue)
    if (cond.kind === 'some') {
      const value = this.expr(cond.place)
      const bind = this.bindName(cond.place)
      if (cond.present) {
        const some = this.narrowed(cond.place, { kind: 'some', bind }, bind, inner)
        return { expr: 'if-some', value, bind, some, none: this.expr(whenFalse) }
      }
      const some = this.narrowed(cond.place, { kind: 'some', bind }, bind, () =>
        this.expr(whenFalse),
      )
      return { expr: 'if-some', value, bind, some, none: inner() }
    }
    if (cond.kind === 'variant') {
      const value = this.expr(cond.place)
      const binds = new Map<string, string>()
      const narrowing: Narrowing = { kind: 'variant', name: cond.name, binds }
      const then = cond.is
        ? this.narrowed(cond.place, narrowing, undefined, inner)
        : this.narrowed(cond.place, narrowing, undefined, () => this.expr(whenFalse))
      const otherwise = cond.is ? this.expr(whenFalse) : inner()
      const expr: Expr = { expr: 'if-variant', value, variant: cond.name, then, else: otherwise }
      if (binds.size) expr.binds = Object.fromEntries(binds)
      return expr
    }
    const plain = [first]
    while (rest.length && this.split(rest[0] as ts.Expression).kind === 'bool') {
      plain.push(rest.shift() as ts.Expression)
    }
    const test = plain
      .map((expression) => this.expr(expression))
      .reduce((left, right) => ({ expr: 'binary', op: 'and', left, right }))
    return { expr: 'ternary', cond: test, then: inner(), else: this.expr(whenFalse) }
  }

  private object(node: ts.ObjectLiteralExpression): Expr {
    const contextual = this.types.checker.getContextualType(node)
    if (!contextual) throw new CompileError(node, 'an object literal needs a type from its context')
    const union = this.types.unionOf(contextual)
    const fields: Record<string, Expr> = {}
    let kind: string | undefined
    for (const property of node.properties) {
      let name: string
      let value: ts.Expression
      if (ts.isPropertyAssignment(property) && ts.isIdentifier(property.name)) {
        name = property.name.text
        value = property.initializer
      } else if (ts.isShorthandPropertyAssignment(property)) {
        name = property.name.text
        value = property.name
      } else {
        throw new CompileError(property, 'a record is built from named properties')
      }
      if (union !== undefined && name === 'kind') {
        const literal = unparen(value)
        if (!ts.isStringLiteral(literal)) throw new CompileError(value, 'the kind is a literal')
        kind = literal.text
        continue
      }
      fields[name] = this.expr(value)
    }
    if (union !== undefined) {
      if (kind === undefined) throw new CompileError(node, `a ${union} says its kind`)
      const expr: Expr = { expr: 'variant', type: union, kind }
      if (Object.keys(fields).length) expr.fields = fields
      return expr
    }
    const awaited = this.types.checker.getAwaitedType(contextual) ?? contextual
    const record = this.types.map(this.types.checker.getNonNullableType(awaited))
    if (typeof record !== 'string' || !this.module.types.has(record)) {
      throw new CompileError(
        node,
        `an object literal makes a record or a union variant, not a ${this.types.checker.typeToString(contextual)}`,
      )
    }
    return { expr: 'record', type: record, fields }
  }
}

// Helpers

function unparen(node: ts.Expression): ts.Expression {
  return ts.isParenthesizedExpression(node) ? unparen(node.expression) : node
}

function isUndefined(node: ts.Expression): boolean {
  return ts.isIdentifier(node) && node.text === 'undefined'
}

function isLiteral(node: ts.Expression): boolean {
  return (
    ts.isNumericLiteral(node) ||
    node.kind === ts.SyntaxKind.TrueKeyword ||
    node.kind === ts.SyntaxKind.FalseKeyword
  )
}

function isBoolean(node: ts.Expression): boolean {
  return node.kind === ts.SyntaxKind.TrueKeyword || node.kind === ts.SyntaxKind.FalseKeyword
}

function isReference(node: ts.Expression): boolean {
  return ts.isIdentifier(node) || ts.isPropertyAccessExpression(node)
}

function numeric(node: ts.NumericLiteral): number {
  return Number(node.text.replace(/_/g, ''))
}

function label(node: ts.Identifier | undefined): { label?: string } {
  return node ? { label: node.text } : {}
}

/** The conjuncts of `a && b && c`, left to right. */
function conjuncts(node: ts.Expression): ts.Expression[] {
  const bare = unparen(node)
  if (
    ts.isBinaryExpression(bare) &&
    bare.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken
  ) {
    return [...conjuncts(bare.left), ...conjuncts(bare.right)]
  }
  return [bare]
}

/** The disjuncts of `a || b || c`, left to right. */
function disjuncts(node: ts.Expression): ts.Expression[] {
  const bare = unparen(node)
  if (ts.isBinaryExpression(bare) && bare.operatorToken.kind === ts.SyntaxKind.BarBarToken) {
    return [...disjuncts(bare.left), ...disjuncts(bare.right)]
  }
  return [bare]
}

/** `x++`, `++x`, `x--`: the place and the step. */
function incrementOf(node: ts.Expression): { target: ts.Expression; op: BinaryOp } | undefined {
  if (ts.isPostfixUnaryExpression(node) || ts.isPrefixUnaryExpression(node)) {
    if (node.operator === ts.SyntaxKind.PlusPlusToken) return { target: node.operand, op: 'add' }
    if (node.operator === ts.SyntaxKind.MinusMinusToken) return { target: node.operand, op: 'sub' }
  }
  if (
    ts.isBinaryExpression(node) &&
    node.operatorToken.kind === ts.SyntaxKind.PlusEqualsToken &&
    ts.isNumericLiteral(node.right) &&
    numeric(node.right) === 1
  ) {
    return { target: node.left, op: 'add' }
  }
  return undefined
}

/** Whether a statement leaves its block on every path. */
function leaves(node: ts.Statement): boolean {
  if (ts.isReturnStatement(node) || ts.isBreakStatement(node) || ts.isContinueStatement(node))
    return true
  if (ts.isBlock(node)) {
    const last = node.statements[node.statements.length - 1]
    return last !== undefined && leaves(last)
  }
  if (ts.isIfStatement(node)) {
    return (
      node.elseStatement !== undefined && leaves(node.thenStatement) && leaves(node.elseStatement)
    )
  }
  return false
}

/** Whether an IR expression is a place. */
export function isPlace(expr: Expr): boolean {
  switch (expr.expr) {
    case 'local':
    case 'self':
      return true
    case 'field':
    case 'index':
      return isPlace(expr.of)
    default:
      return false
  }
}
