// Copyright 2026 the Exedra Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

declare module "*?highlighted" {
  /** Escaped, syntax-highlighted HTML generated from the actual source file. */
  const html: string;
  export default html;
  /** Focused excerpts of actual author functions and their local dependencies. */
  export const authorSections: Readonly<
    Record<
      string,
      {
        readonly html: string;
        readonly declarations: readonly string[];
      }
    >
  >;
}
