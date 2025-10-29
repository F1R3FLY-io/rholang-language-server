; Minimal highlights.scm for Rholang Tree-Sitter
; This is a simplified version that works with the current grammar

; Comments
(line_comment) @comment
(block_comment) @comment

; Keywords
[
  "contract"
  "for"
  "in"
  "if"
  "else"
  "match"
  "select"
  "new"
  "let"
] @keyword

; Boolean literals
[
  "true"
  "false"
] @boolean

; Operators
[
  "+"
  "-"
  "*"
  "/"
  "++"
  "--"
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "and"
  "or"
  "not"
  "matches"
] @operator

; Delimiters
[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

[
  ";"
  ","
  "."
] @punctuation.delimiter

; Special operators
"@" @operator.special

; Contract names
(contract
  name: (var) @function)

; Variable references
(var) @variable

; String literals
(string_literal) @string

; Integer literals
(long_literal) @number

; Boolean literals
(bool_literal) @boolean

; Bundle modifiers
[
  (bundle_write)
  (bundle_read)
  (bundle_equiv)
  (bundle_read_write)
] @keyword.modifier

; Bundle nodes
(bundle) @keyword
