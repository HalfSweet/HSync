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
    let symbol =
      ts.isIdentifier(node) &&
      ts.isShorthandPropertyAssignment(node.parent) &&
      node.parent.name === node
        ? this.checker.getShorthandAssignmentValueSymbol(node.parent)
        : this.checker.getSymbolAtLocation(node)
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
    const numeric = this.numericEnumOf(bare)
    if (numeric) return numeric
    const members = bare.isUnion() ? bare.types : [bare]
    if (!members.every((member) => member.isStringLiteral())) return undefined
    return this.isUser(bare.aliasSymbol) ? bare.aliasSymbol?.name : undefined
  }

  /** A numeric enum retains its declared names independently of its numeric wire values. */
  numericEnumOf(type: ts.Type): string | undefined {
    const declaration = this.aliasDeclaration(type)
    if (!declaration) return undefined
    const node = unparenthesize(declaration.type)
    if (this.isNumericEnumHelper(node)) return declaration.name.text
    if (ts.isUnionTypeNode(node) && node.types.map(unparenthesize).every(isNumericType)) {
      return declaration.name.text
    }
    return undefined
  }

  /** An externally tagged enum mixes unit strings with single-property payload objects. */
  externalEnumOf(type: ts.Type): string | undefined {
    const declaration = this.aliasDeclaration(type)
    if (!declaration) return undefined
    const node = unparenthesize(declaration.type)
    if (!ts.isUnionTypeNode(node)) return undefined
    const members = node.types.map(unparenthesize)
    const hasUnit = members.some(isStringType)
    const hasPayload = members.some((member) => ts.isTypeLiteralNode(member))
    return hasUnit &&
      hasPayload &&
      members.every(
        (member) =>
          isStringType(member) ||
          (ts.isTypeLiteralNode(member) &&
            member.members.length === 1 &&
            ts.isPropertySignature(member.members[0] as ts.Node) &&
            (member.members[0] as ts.PropertySignature).name.getText() !== 'kind'),
      )
      ? declaration.name.text
      : undefined
  }

  /** A numeric literal's logical variant name in its contextual enum. */
  numericVariant(type: ts.Type, value: number): { type: string; name: string } | undefined {
    const declaration = this.aliasDeclaration(type)
    if (!declaration || !this.numericEnumOf(type)) return undefined
    const def = this.alias(declaration)
    if (def?.type !== 'numeric-enum') return undefined
    const variant = def.variants.find((variant) => variant.value === value)
    return variant ? { type: def.name, name: variant.name } : undefined
  }

  private aliasDeclaration(type: ts.Type): ts.TypeAliasDeclaration | undefined {
    const bare = this.checker.getNonNullableType(type)
    if (!this.isUser(bare.aliasSymbol)) return undefined
    return bare.aliasSymbol?.declarations?.find(ts.isTypeAliasDeclaration)
  }

  private isNumericEnumHelper(node: ts.TypeNode): node is ts.TypeReferenceNode {
    return ts.isTypeReferenceNode(node) && this.libName(node.typeName) === 'NumericEnum'
  }

  /** Whether a type directly marks a field's hexadecimal JSON representation. */
  isHex(type: ts.Type): boolean {
    return type
      .getProperties()
      .some((property) => String(property.escapedName).startsWith('__@hex'))
  }

  /** Hexadecimal representation belongs to a direct u64 field, never a nested value type. */
  checkHex(type: ts.Type, at: ts.Node, field = false): void {
    const contains = (part: ts.Type): boolean => {
      if (this.isHex(part)) return true
      if (part.isUnion()) return part.types.some(contains)
      if (this.checker.isTupleType(part)) {
        return this.checker.getTypeArguments(part as ts.TypeReference).some(contains)
      }
      if (part.aliasTypeArguments) return part.aliasTypeArguments.some(contains)
      return false
    }
    if (contains(type) && !(field && this.isHex(type) && this.widthOf(type) === 64)) {
      throw new CompileError(at, 'HexU64 is only supported as the direct type of a u64 field')
    }
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
    const external = this.externalEnumOf(bare)
    if (external !== undefined) return external
    if (bare.flags & ts.TypeFlags.StringLike) return 'text'
    const union = this.unionOf(bare)
    if (union !== undefined) return union
    if (this.checker.isTupleType(bare)) {
      const tuple = bare as ts.TupleTypeReference
      const members = this.checker.getTypeArguments(tuple)
      if (
        !members.length ||
        members.length > 32 ||
        tuple.target.elementFlags.some((flag) => flag !== ts.ElementFlags.Required)
      ) {
        return undefined
      }
      const mapped = members.map((member) => this.map(member))
      const first = mapped[0]
      if (
        first === undefined ||
        mapped.some((member) => JSON.stringify(member) !== JSON.stringify(first))
      ) {
        return undefined
      }
      return { fixedArray: first, length: members.length }
    }
    if (bare.aliasSymbol && this.isLib(bare.aliasSymbol) && bare.aliasSymbol.name === 'Bounded') {
      const [item, bound] = bare.aliasTypeArguments ?? []
      const itemType = item === undefined ? undefined : this.map(item)
      if (
        itemType === undefined ||
        bound === undefined ||
        !bound.isNumberLiteral() ||
        !Number.isSafeInteger(bound.value) ||
        bound.value < 1 ||
        bound.value > 0xffff_ffff
      ) {
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
  mapNode(node: ts.TypeNode, field = false): Type {
    const type = this.checker.getTypeFromTypeNode(node)
    this.checkHex(type, node, field)
    this.checkFixedArray(type, node)
    const mapped = this.map(type)
    if (mapped === undefined) {
      throw new CompileError(
        node,
        `the type ${node.getText()} has no IR type; name a record or give an integer its width`,
      )
    }
    return mapped
  }

  /** Native serde array derives support exact arrays through length 32. */
  checkFixedArray(type: ts.Type, at: ts.Node): void {
    const bare = this.checker.getNonNullableType(type)
    if (this.checker.isTupleType(bare)) {
      const members = this.checker.getTypeArguments(bare as ts.TupleTypeReference)
      if (members.length < 1 || members.length > 32) {
        throw new CompileError(
          at,
          'a fixed array has no IR type outside lengths 1..32, the native serde array limit',
        )
      }
    }
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
        this.checkFixedArray(this.checker.getTypeFromTypeNode(statement.type), statement.type)
        const def = this.alias(statement)
        if (def) defs.push(def)
      }
    }
    return defs
  }

  private alias(statement: ts.TypeAliasDeclaration): TypeDef | undefined {
    const name = statement.name.text
    const node = unparenthesize(statement.type)
    if (this.isNumericEnumHelper(node)) {
      const values = node.typeArguments?.[0]
      if (!values || !ts.isTypeLiteralNode(values)) {
        throw new CompileError(node, 'NumericEnum declares named numeric literal properties')
      }
      const variants = values.members.map((member) => {
        if (
          !ts.isPropertySignature(member) ||
          !ts.isIdentifier(member.name) ||
          !member.type ||
          !isNumericType(member.type) ||
          member.questionToken
        ) {
          throw new CompileError(
            member,
            'NumericEnum variants have names and unsigned integer literals',
          )
        }
        return { name: member.name.text, value: numericTypeValue(member.type) }
      })
      this.checkNumericVariants(variants, node)
      return { name, type: 'numeric-enum', variants }
    }
    if (ts.isTypeLiteralNode(node)) {
      const variant = this.variant(node)
      if (variant) return { name, type: 'union', variants: [variant] }
      return { name, type: 'record', fields: this.fields(node.members, node) }
    }
    if (ts.isUnionTypeNode(node)) {
      const members = node.types.map(unparenthesize)
      if (members.every(isNumericType)) {
        const variants = members.map((member) => {
          const value = numericTypeValue(member)
          return { name: `value${value}`, value }
        })
        this.checkNumericVariants(variants, node)
        return { name, type: 'numeric-enum', variants }
      }
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
      if (this.externalEnumOf(this.checker.getTypeFromTypeNode(node))) {
        const variants = members.map((member) => {
          if (isStringType(member)) return { name: (member.literal as ts.StringLiteral).text }
          const field = this.fields((member as ts.TypeLiteralNode).members, member)[0] as Field
          if (field.json)
            throw new CompileError(member, 'an external enum payload cannot use HexU64')
          return { name: field.name, payload: field.type }
        })
        if (new Set(variants.map((variant) => variant.name)).size !== variants.length) {
          throw new CompileError(node, 'external enum variant names are unique')
        }
        return { name, type: 'external-enum', variants }
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

  private checkNumericVariants(variants: { name: string; value: number }[], at: ts.Node): void {
    if (
      !variants.length ||
      variants.some((variant) => !Number.isSafeInteger(variant.value) || variant.value < 0)
    ) {
      throw new CompileError(at, 'numeric enums have unsigned safe integer wire values')
    }
    if (
      new Set(variants.map((variant) => variant.value)).size !== variants.length ||
      new Set(variants.map((variant) => variant.name)).size !== variants.length
    ) {
      throw new CompileError(at, 'numeric enum names and wire values are unique')
    }
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
      const field: Field = { name, type: this.mapNode(member.type, true) }
      if (this.isHex(this.checker.getTypeFromTypeNode(member.type))) field.json = 'hex-u64'
      if (this.isChannelNode(member.type)) field.channel = true
      return field
    })
  }
}

function unparenthesize(node: ts.TypeNode): ts.TypeNode {
  return ts.isParenthesizedTypeNode(node) ? unparenthesize(node.type) : node
}

function isStringType(node: ts.TypeNode): node is ts.LiteralTypeNode {
  return ts.isLiteralTypeNode(node) && ts.isStringLiteral(node.literal)
}

function isNumericType(node: ts.TypeNode): node is ts.LiteralTypeNode {
  return ts.isLiteralTypeNode(node) && ts.isNumericLiteral(node.literal)
}

function numericTypeValue(node: ts.TypeNode): number {
  return integerLiteral((node as ts.LiteralTypeNode).literal as ts.NumericLiteral)
}

/** JSON-backed IR numbers must retain every bit of the source literal. */
export function integerLiteral(node: ts.NumericLiteral): number {
  const value = Number(node.text.replace(/_/g, ''))
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new CompileError(
      node,
      'an integer literal must be an unsigned safe integer; assemble wider u64 values with shifts',
    )
  }
  return value
}
