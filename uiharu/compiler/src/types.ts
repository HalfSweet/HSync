// SPDX-License-Identifier: Apache-2.0

/**
 * From TypeScript's types to the IR's: integer widths read from the brand of `u8` … `u64`,
 * the DSL's own interfaces mapped to the runtime's types, declared records, enums and unions
 * by name; and the collection of a file's type declarations as IR type definitions.
 */

import ts from 'typescript'

import type { Field, Type, TypeDef, UnionVariant } from './ir.ts'
import { CompileError } from './program.ts'

/** The runtime's types, by the name of the DSL interface that declares them. */
const RUNTIME_TYPES: Record<string, string> = {
  Provenance: 'provenance',
  Range: 'range',
  Levels: 'levels',
  ChannelMask: 'mask',
  Transaction: 'cursor',
}

/** Maps TypeScript types to IR types and collects type declarations. */
export class Types {
  readonly program: ts.Program
  readonly checker: ts.TypeChecker
  readonly lib: ts.SourceFile

  constructor(program: ts.Program, lib: ts.SourceFile) {
    this.program = program
    this.checker = program.getTypeChecker()
    this.lib = lib
  }

  /** Whether the symbol is declared in a decoder module: not by TypeScript's own libraries
   * and not by the DSL's declarations. */
  isUser(symbol: ts.Symbol | undefined): boolean {
    const declarations = symbol?.declarations ?? []
    return (
      declarations.length > 0 &&
      declarations.every(
        (decl) =>
          decl.getSourceFile() !== this.lib &&
          !this.program.isSourceFileDefaultLibrary(decl.getSourceFile()),
      )
    )
  }

  /** The symbol an identifier refers to, through import aliases. */
  symbolOf(node: ts.Node): ts.Symbol | undefined {
    let symbol = this.checker.getSymbolAtLocation(node)
    if (symbol && symbol.flags & ts.SymbolFlags.Alias) {
      symbol = this.checker.getAliasedSymbol(symbol)
    }
    return symbol
  }

  /** Whether the symbol is declared by the DSL's own declarations. */
  isLib(symbol: ts.Symbol | undefined): boolean {
    return symbol?.declarations?.some((decl) => decl.getSourceFile() === this.lib) ?? false
  }

  /** The DSL declaration's name, when `node` refers to one. */
  libName(node: ts.Node): string | undefined {
    const symbol = this.symbolOf(node)
    return this.isLib(symbol) ? symbol?.name : undefined
  }

  /** The declared type of an expression: for a reference, the type it was declared with, not
   * the one it is narrowed to here. */
  declaredType(node: ts.Expression): ts.Type {
    const target = ts.isPropertyAccessExpression(node) ? node.name : node
    if (ts.isIdentifier(target)) {
      const symbol = this.symbolOf(target)
      if (symbol && symbol.declarations?.length) {
        return this.checker.getTypeOfSymbol(symbol)
      }
    }
    return this.checker.getTypeAtLocation(node)
  }

  /** The width of an integer type, from the brand of `u8` … `u64`. */
  widthOf(type: ts.Type): 8 | 16 | 32 | 64 | undefined {
    const parts = type.isIntersection() ? type.types : [type]
    if (!parts.some((part) => part.flags & ts.TypeFlags.Number)) return undefined
    let width: 8 | 16 | 32 | 64 | undefined
    for (const part of parts) {
      if (!(part.flags & ts.TypeFlags.Object)) continue
      for (const property of part.getProperties()) {
        if (!String(property.escapedName).startsWith('__@width')) continue
        const brand = this.checker.getTypeOfSymbol(property)
        for (const literal of brand.isUnion() ? brand.types : [brand]) {
          if (literal.isNumberLiteral()) {
            const value = literal.value as 8 | 16 | 32 | 64
            if (width === undefined || value > width) width = value
          }
        }
      }
    }
    return width
  }

  /** Whether the type is `Channel`. */
  isChannel(type: ts.Type): boolean {
    const parts = type.isIntersection() ? type.types : [type]
    return parts.some(
      (part) =>
        part.flags & ts.TypeFlags.Object &&
        part
          .getProperties()
          .some((property) => String(property.escapedName).startsWith('__@channel')),
    )
  }

  /** The name of the enum a type is, for a union of string literals declared under a name. */
  enumOf(type: ts.Type): string | undefined {
    const bare = this.checker.getNonNullableType(type)
    const members = bare.isUnion() ? bare.types : [bare]
    if (!members.every((member) => member.isStringLiteral())) return undefined
    return this.isUser(bare.aliasSymbol) ? bare.aliasSymbol?.name : undefined
  }

  /** Whether a type is object types tagged by `kind`, declared under a name or not (the items
   * a frame cursor hands out are such a union without a name). */
  isTaggedUnion(type: ts.Type): boolean {
    const bare = this.checker.getNonNullableType(type)
    const members = bare.isUnion() ? bare.types : [bare]
    return members.every(
      (member) =>
        member.flags & ts.TypeFlags.Object &&
        member.getProperty('kind') !== undefined &&
        !this.isLib(member.symbol),
    )
  }

  /** The name of the union a type is: object types tagged by `kind`, declared under a name. */
  unionOf(type: ts.Type): string | undefined {
    const bare = this.checker.getNonNullableType(type)
    const members = bare.isUnion() ? bare.types : [bare]
    const tagged = members.every(
      (member) =>
        member.flags & ts.TypeFlags.Object &&
        member.getProperty('kind') !== undefined &&
        !this.isLib(member.symbol),
    )
    return tagged && this.isUser(bare.aliasSymbol) ? bare.aliasSymbol?.name : undefined
  }

  /** The IR type of a TypeScript type, or `undefined` when it has none: a plain `number`
   * without a width, or a record the runtime's operations hand out. */
  map(type: ts.Type): Type | undefined {
    const bare = this.checker.getNonNullableType(type)
    if (bare !== type && !(type.flags & ts.TypeFlags.Undefined)) {
      // `T | undefined`: an option of T, unless T itself is nothing.
      const inner = this.map(bare)
      return inner === undefined ? undefined : { option: inner }
    }
    if (bare.flags & (ts.TypeFlags.BooleanLike | ts.TypeFlags.BooleanLiteral)) return 'bool'
    if (bare.isUnion() && bare.types.every((t) => t.flags & ts.TypeFlags.BooleanLiteral)) {
      return 'bool'
    }
    const width = this.widthOf(bare)
    if (width !== undefined) return `u${width}`
    const enumName = this.enumOf(bare)
    if (enumName !== undefined) return enumName
    if (bare.flags & ts.TypeFlags.StringLike) return 'text'
    const union = this.unionOf(bare)
    if (union !== undefined) return union
    if (bare.aliasSymbol && this.isLib(bare.aliasSymbol) && bare.aliasSymbol.name === 'Bounded') {
      const [item, bound] = bare.aliasTypeArguments ?? []
      const itemType = item === undefined ? undefined : this.map(item)
      if (itemType === undefined || bound === undefined || !bound.isNumberLiteral()) {
        return undefined
      }
      return { array: itemType, bound: bound.value }
    }
    if (bare.flags & ts.TypeFlags.Object) {
      const symbol = bare.symbol
      if (this.isLib(symbol)) {
        const runtime = RUNTIME_TYPES[symbol.name]
        if (runtime !== undefined) return runtime
        if (symbol.name === 'Batch') {
          const [item] = this.checker.getTypeArguments(bare as ts.TypeReference)
          const itemType = item === undefined ? undefined : this.map(item)
          // The bound is the batch's capacity, which the initializer says.
          return itemType === undefined ? undefined : { batch: itemType, bound: 0 }
        }
        return undefined
      }
      if (this.isUser(bare.aliasSymbol)) return bare.aliasSymbol?.name
      if (this.isUser(symbol)) return symbol.name
    }
    return undefined
  }

  /** The IR type of a type annotation, which must have one. */
  mapNode(node: ts.TypeNode): Type {
    const mapped = this.map(this.checker.getTypeFromTypeNode(node))
    if (mapped === undefined) {
      throw new CompileError(
        node,
        `the type ${node.getText()} has no IR type; name a record or give an integer its width`,
      )
    }
    return mapped
  }

  /** Whether a type annotation names `Channel`. */
  /** Whether the type names channels of the source: `Channel`, `Channel | undefined` for a
   * line that may be left unconnected, or `Bounded<Channel, N>` for a list of them. */
  isChannelNode(node: ts.TypeNode | undefined): boolean {
    if (node === undefined) return false
    let type = this.checker.getNonNullableType(this.checker.getTypeFromTypeNode(node))
    if (type.aliasSymbol && this.isLib(type.aliasSymbol) && type.aliasSymbol.name === 'Bounded') {
      type = type.aliasTypeArguments?.[0] ?? type
    }
    return this.isChannel(type)
  }

  /** The type definitions declared at the top of `file`, in order: interfaces as records,
   * unions of string literals as enums, object types tagged by `kind` as unions. Aliases of
   * other types (`type TxId = u64`) define nothing: they are resolved where they are used. */
  collect(file: ts.SourceFile): TypeDef[] {
    const defs: TypeDef[] = []
    for (const statement of file.statements) {
      if (ts.isInterfaceDeclaration(statement)) {
        defs.push({
          name: statement.name.text,
          type: 'record',
          fields: this.fields(statement.members, statement),
        })
      } else if (ts.isTypeAliasDeclaration(statement)) {
        const def = this.alias(statement)
        if (def) defs.push(def)
      }
    }
    return defs
  }

  private alias(statement: ts.TypeAliasDeclaration): TypeDef | undefined {
    const name = statement.name.text
    const node = statement.type
    if (ts.isTypeLiteralNode(node)) {
      const variant = this.variant(node)
      if (variant) return { name, type: 'union', variants: [variant] }
      return { name, type: 'record', fields: this.fields(node.members, node) }
    }
    if (ts.isUnionTypeNode(node)) {
      const members = node.types.map(unparenthesize)
      if (
        members.every(
          (member) => ts.isLiteralTypeNode(member) && ts.isStringLiteral(member.literal),
        )
      ) {
        return {
          name,
          type: 'enum',
          variants: members.map(
            (member) => ((member as ts.LiteralTypeNode).literal as ts.StringLiteral).text,
          ),
        }
      }
      if (members.every((member) => ts.isTypeLiteralNode(member))) {
        const variants = members.map((member) => {
          const variant = this.variant(member)
          if (!variant) throw new CompileError(member, 'every member of a union has a kind')
          return variant
        })
        return { name, type: 'union', variants }
      }
    }
    return undefined
  }

  /** The variant an object type with a `kind` literal is; `undefined` without one. */
  private variant(node: ts.TypeLiteralNode): UnionVariant | undefined {
    const kind = node.members.find(
      (member) =>
        ts.isPropertySignature(member) &&
        ts.isIdentifier(member.name) &&
        member.name.text === 'kind',
    ) as ts.PropertySignature | undefined
    if (!kind?.type || !ts.isLiteralTypeNode(kind.type) || !ts.isStringLiteral(kind.type.literal)) {
      return undefined
    }
    const fields = this.fields(
      node.members.filter((member) => member !== kind),
      node,
    )
    return fields.length
      ? { name: kind.type.literal.text, fields }
      : { name: kind.type.literal.text }
  }

  private fields(members: readonly ts.TypeElement[], at: ts.Node): Field[] {
    return members.map((member) => {
      if (!ts.isPropertySignature(member) || !member.type) {
        throw new CompileError(member ?? at, 'a record has typed properties only')
      }
      if (member.questionToken) {
        throw new CompileError(member, 'write an optional field as `T | undefined`')
      }
      const name = ts.isIdentifier(member.name) ? member.name.text : undefined
      if (name === undefined) throw new CompileError(member, 'a field has a plain name')
      const field: Field = { name, type: this.mapNode(member.type) }
      if (this.isChannelNode(member.type)) field.channel = true
      return field
    })
  }
}

function unparenthesize(node: ts.TypeNode): ts.TypeNode {
  return ts.isParenthesizedTypeNode(node) ? unparenthesize(node.type) : node
}
