// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import ts from "typescript";

function escapeHtml(source) {
  return source.replace(/[&<>"'\r]/g, (character) => {
    return {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
      "\r": "&#13;",
    }[character];
  });
}

function tokenClass(kind) {
  if (kind >= ts.SyntaxKind.FirstKeyword && kind <= ts.SyntaxKind.LastKeyword)
    return "keyword";
  if (
    kind === ts.SyntaxKind.StringLiteral ||
    kind === ts.SyntaxKind.RegularExpressionLiteral ||
    (kind >= ts.SyntaxKind.FirstTemplateToken &&
      kind <= ts.SyntaxKind.LastTemplateToken)
  )
    return "string";
  if (
    kind === ts.SyntaxKind.NumericLiteral ||
    kind === ts.SyntaxKind.BigIntLiteral
  )
    return "number";
  if (
    kind === ts.SyntaxKind.SingleLineCommentTrivia ||
    kind === ts.SyntaxKind.MultiLineCommentTrivia
  )
    return "comment";
  if (
    kind >= ts.SyntaxKind.FirstPunctuation &&
    kind <= ts.SyntaxKind.LastPunctuation
  )
    return "punctuation";
  return undefined;
}

/** Highlight compiler token spans without changing a single source character. */
export function highlightTypeScript(source) {
  const file = ts.createSourceFile(
    "sample.ts",
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
  const tokens = [];
  function visit(node) {
    // JSDoc is trivia; scanning its whole range keeps embedded markup a comment.
    if (node.kind === ts.SyntaxKind.JSDocComment) return;
    if (ts.isToken(node)) {
      const start = node.getStart(file);
      if (node.end > start)
        tokens.push({ start, end: node.end, kind: node.kind });
      return;
    }
    for (const child of node.getChildren(file)) visit(child);
  }
  visit(file);

  const html = [];
  function emit(text, kind) {
    const escaped = escapeHtml(text);
    const style = tokenClass(kind);
    html.push(style ? `<span class="ts-${style}">${escaped}</span>` : escaped);
  }
  function trivia(start, end) {
    const scanner = ts.createScanner(
      ts.ScriptTarget.Latest,
      false,
      ts.LanguageVariant.Standard,
      source,
      undefined,
      start,
      end - start,
    );
    for (
      let kind = scanner.scan();
      kind !== ts.SyntaxKind.EndOfFileToken;
      kind = scanner.scan()
    ) {
      emit(scanner.getTokenText(), kind);
    }
  }

  let position = 0;
  for (const token of tokens) {
    trivia(position, token.start);
    emit(source.slice(token.start, token.end), token.kind);
    position = token.end;
  }
  trivia(position, source.length);
  return html.join("");
}

/** Build focused author excerpts from actual declarations, including their local
 * type/helper dependencies. The complete source is still highlighted separately.
 */
export function highlightAuthorSections(source) {
  const file = ts.createSourceFile(
    "sample.ts",
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TS,
  );
  const declarations = new Map();
  for (const node of file.statements) {
    if (
      ((ts.isFunctionDeclaration(node) && node.body) ||
        ts.isInterfaceDeclaration(node) ||
        ts.isTypeAliasDeclaration(node)) &&
      node.name
    ) {
      declarations.set(node.name.text, node);
    }
  }
  const sections = {};
  for (const [name, author] of declarations) {
    if (!ts.isFunctionDeclaration(author) || !name.startsWith("author"))
      continue;
    const needed = new Set([name]);
    function collect(node) {
      function visit(child) {
        if (
          ts.isIdentifier(child) &&
          declarations.has(child.text) &&
          !needed.has(child.text)
        ) {
          needed.add(child.text);
          collect(declarations.get(child.text));
        }
        ts.forEachChild(child, visit);
      }
      ts.forEachChild(node, visit);
    }
    collect(author);
    const types = [],
      helpers = [];
    for (const [dependency, node] of declarations) {
      if (!needed.has(dependency) || dependency === name) continue;
      (ts.isFunctionDeclaration(node) ? helpers : types).push(node);
    }
    const nodes = [...types, author, ...helpers];
    const excerpt = nodes
      .map((node) => source.slice(node.getFullStart(), node.end).trim())
      .join("\n\n");
    sections[name] = {
      html: highlightTypeScript(excerpt),
      declarations: nodes.map((node) => node.name.text),
    };
  }
  return sections;
}
