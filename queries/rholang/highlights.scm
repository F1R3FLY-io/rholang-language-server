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
  "bundle"
] @keyword

; Bundle keywords
[
  (bundle_write)
  (bundle_read)
  (bundle_equiv)
  (bundle_read_write)
] @keyword.modifier

; Word-based operators (logical/keyword-like)
[
  "or"
  "and"
  "matches"
  "not"
] @keyword.operator

; Symbolic operators
[
  "|"
  "!?"
  (send_single)
  (send_multiple)
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "+"
  "++"
  "-"
  "--"
  "*"
  "/"
  "%"
  "%%"
  "~"
  "\\/"
  "/\\"
  "<-"
  "<<-"
  "<="
  "?!"
  "=>"
  ":"
  "="
  "&"
] @operator

; Literals
(bool_literal) @boolean
(long_literal) @number
(string_literal) @string
(uri_literal) @string.special
(nil) @constant.builtin
(unit) @constant.builtin

; Types
(simple_type) @type.builtin

; Additional operators in context
(quote "@" @operator)
(eval "*" @operator)
(var_ref_kind) @operator

; Punctuation (split for brackets and delimiters)
[
  "("
  ")"
  "{"
  "}"
  "["
  "]"
] @punctuation.bracket

[
  ","
  ";"
  "."
  "..."
] @punctuation.delimiter

; Pathmap specific delimiters
(pathmap "{|" @punctuation.bracket)
(pathmap "|}" @punctuation.bracket)

; Collections keyword
(set "Set" @type.builtin)

; Variables and Names
(var) @variable
(wildcard) @variable.builtin
(var_ref var: (var) @variable)

; Contract and function names
(contract name: (quote) @function)
(contract name: (var) @function)
(contract name: (wildcard) @function)

; Method names
(method name: (var) @function.method)
(method receiver: (_) @variable)

; Name declarations
(name_decl (var) @variable)
(name_decl uri: (uri_literal) @string.special)

; Receipts and bindings
(linear_bind names: (names) @variable)
(repeated_bind names: (names) @variable)
(peek_bind names: (names) @variable)

; Let declarations
(decl names: (names) @variable)

; Case and branch patterns
(case pattern: (_) @variable)
(branch pattern: (_) @variable)

; Key-value pairs in maps
(key_value_pair key: (_) @property)
(key_value_pair value: (_))

; Remainder patterns
(_proc_remainder remainder: (_) @variable)
(_name_remainder cont: (_) @variable)

; Channels (quotes and evals)
(quote) @function.call
(eval name: (_) @variable)

; Send and receive
(send channel: (_) @function.call)
(send_sync channel: (_) @function.call)
(input receipts: (receipts))

; Source channels in bindings
(linear_bind input: (_) @function.call)
(repeated_bind input: (name) @function.call)
(peek_bind input: (name) @function.call)

; Field labels for better semantic understanding
(ifElse condition: (_))
(ifElse consequence: (_))
(ifElse alternative: (_))

(match expression: (_))

(new decls: (name_decls))
(new proc: (_))

(let decls: (_))
(let proc: (_))

(bundle proc: (_))
