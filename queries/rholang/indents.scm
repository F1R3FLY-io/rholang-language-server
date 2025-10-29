; Indent blocks and collections
[
  (block)
  (list)
  (tuple)
  (set)
  (map)
  (pathmap)
] @indent

; Indent control structures
[
  (ifElse)
  (match)
  (choice)
  (contract)
  (input)
  (new)
  (let)
  (bundle)
] @indent

; Indent messages/inputs
[
  (inputs)
  (messages)
  (args)
] @indent

; Indent case and branch bodies
(case) @indent
(branch) @indent

; Indent receipts and bindings
(receipts) @indent
(linear_bind) @indent
(repeated_bind) @indent
(peek_bind) @indent

; Indent declarations
(name_decls) @indent
(linear_decls) @indent
(conc_decls) @indent

; Branch points - align with delimiter
(par "|" @branch)
(branch "=>" @branch)
(case "=>" @branch)

; Dedent closing brackets
[
  "}"
  "]"
  ")"
] @dedent

; Auto-dedent after continuation
(empty_cont) @dedent
(non_empty_cont) @indent
