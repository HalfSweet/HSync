// SPDX-License-Identifier: Apache-2.0

/**
 * The decoder IR as `uiharu-ir` reads it (`crates/uiharu-ir/src/model.rs`): the JSON these
 * types serialize to is the interface between the compiler and the backend. `docs/dsl.md` §4
 * describes it; keys that are `undefined` are left out of the JSON.
 */

export type Type =
  string | { option: Type } | { array: Type; bound: number } | { batch: Type; bound: number }

export interface Field {
  name: string
  type: Type
  channel?: true
}

export interface UnionVariant {
  name: string
  fields?: Field[]
}

export type TypeDef =
  | { name: string; type: 'record'; fields: Field[] }
  | { name: string; type: 'enum'; variants: string[] }
  | { name: string; type: 'union'; variants: UnionVariant[] }

export interface SchemaRef {
  name: string
  version: number
}

export interface Framing {
  begin: string
  item: string
  end: string
  id: string
  partial?: string
  reason: string
  normal: string
  unframed?: number
}

export interface Contract {
  schema: SchemaRef
  event: string
  framing?: Framing
}

export interface Import {
  module: string
  types?: TypeDef[]
  contract: Contract
}

export type Literal = boolean | number | string

export interface Const {
  name: string
  type: Type
  value: Literal
}

export interface Param {
  name: string
  type: Type
  mutable?: true
}

export interface Function {
  name: string
  async?: true
  params?: Param[]
  ret?: Type
  body: Stmt[]
}

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json }

export type Category = 'bus' | 'device' | 'line'

export interface Config {
  type: string
  defaults: Json
  validate?: string
}

export type Input =
  { kind: 'logic' } | { kind: 'stream'; schema: SchemaRef } | { kind: 'bus'; schema: SchemaRef }

export type RequireKind = 'equals' | 'one-of' | 'at-least' | 'at-most' | 'within'

export interface RequireSpec {
  key: string
  requirement: RequireKind
  values: Expr[]
}

export interface PropSpec {
  key: string
  value: Expr
}

export interface Recovery {
  reason: string
  incomplete: string
  truncated: string
  missing: string
  gap: string
  end: string
}

export interface StateField {
  name: string
  type: Type
  init: Expr
  readonly?: true
}

export interface Decoder {
  name: string
  category: Category
  config: Config
  input: Input
  requires?: RequireSpec[]
  output: { schema: SchemaRef }
  props?: PropSpec[]
  partial?: 'drain' | 'read'
  recovery?: Recovery
  state?: StateField[]
  methods: Function[]
}

export interface Module {
  ir: 1
  name: string
  types?: TypeDef[]
  contracts?: Contract[]
  imports?: Import[]
  consts?: Const[]
  functions?: Function[]
  decoder: Decoder
}

export type BinaryOp =
  | 'add'
  | 'sub'
  | 'mul'
  | 'div'
  | 'rem'
  | 'bit-and'
  | 'bit-or'
  | 'bit-xor'
  | 'shl'
  | 'shr'
  | 'eq'
  | 'ne'
  | 'lt'
  | 'le'
  | 'gt'
  | 'ge'
  | 'and'
  | 'or'

export type UnaryOp = 'not' | 'bit-not'

export type Pattern =
  | { pattern: 'variant'; name: string; binds?: Record<string, string> }
  | { pattern: 'values'; values: Literal[] }
  | { pattern: 'default' }

export interface Arm {
  pattern: Pattern
  body: Stmt[]
}

export type Stmt =
  | { stmt: 'let'; name: string; mutable?: true; type?: Type; value: Expr }
  | { stmt: 'let-ref'; name: string; place: Expr }
  | { stmt: 'assign'; target: Expr; op?: BinaryOp; value: Expr }
  | { stmt: 'if'; cond: Expr; then: Stmt[]; else?: Stmt[] }
  | { stmt: 'if-some'; value: Expr; bind: string; borrow?: true; then: Stmt[]; else?: Stmt[] }
  | { stmt: 'let-some'; bind: string; value: Expr; borrow?: true; else: Stmt[] }
  | { stmt: 'match'; value: Expr; arms: Arm[] }
  | { stmt: 'loop'; label?: string; body: Stmt[] }
  | { stmt: 'while'; label?: string; cond: Expr; body: Stmt[] }
  | { stmt: 'for'; label?: string; var: string; from: Expr; to: Expr; body: Stmt[] }
  | { stmt: 'for-each'; label?: string; var: string; array: Expr; body: Stmt[] }
  | { stmt: 'break'; label?: string }
  | { stmt: 'continue'; label?: string }
  | { stmt: 'return'; value?: Expr }
  | { stmt: 'expr'; expr: Expr }

export type Expr =
  | { expr: 'lit'; value: boolean | number }
  | { expr: 'text'; value: string }
  | { expr: 'format'; parts: Expr[] }
  | { expr: 'none' }
  | { expr: 'local'; name: string }
  | { expr: 'self'; field: string }
  | { expr: 'field'; of: Expr; name: string }
  | { expr: 'index'; of: Expr; at: Expr }
  | { expr: 'tag'; value: Expr }
  | { expr: 'unary'; op: UnaryOp; value: Expr }
  | { expr: 'binary'; op: BinaryOp; left: Expr; right: Expr }
  | { expr: 'cast'; value: Expr; to: Type }
  | { expr: 'record'; type: string; fields: Record<string, Expr> }
  | { expr: 'variant'; type: string; kind: string; fields?: Record<string, Expr> }
  | { expr: 'enum'; type: string; value: string }
  | { expr: 'array'; items: Expr[] }
  | { expr: 'call'; name: string; args: Expr[] }
  | { expr: 'method'; name: string; args: Expr[] }
  | { expr: 'intrinsic'; name: string; args: Expr[] }
  | { expr: 'ternary'; cond: Expr; then: Expr; else: Expr }
  | { expr: 'if-some'; value: Expr; bind: string; some: Expr; none: Expr }
  | {
      expr: 'if-variant'
      value: Expr
      variant: string
      binds?: Record<string, string>
      then: Expr
      else: Expr
    }
  | { expr: 'required'; value: Expr }
  | { expr: 'await'; op: Op }

export type Op =
  | { op: 'next-change'; mask: Expr }
  | { op: 'next-change-before'; mask: Expr; until: Expr }
  | { op: 'sample' }
  | { op: 'next' }
  | { op: 'item' }
  | { op: 'item-or-end' }
  | { op: 'drain' }
  | { op: 'emit'; event: Expr; at?: Expr }
  | { op: 'method'; name: string; args: Expr[] }
