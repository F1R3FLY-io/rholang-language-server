; Text objects - semantic units for navigation and selection

; Functions (contracts are function-like in Rholang)
(contract) @function.outer
(contract
  proc: (block) @function.inner)

; Blocks
(block) @block.outer

; Parameters
(contract
  formals: (names) @parameter.outer)

; Single parameter in names
(names
  (name) @parameter.inner)

; Method call receiver and arguments
(method) @call.outer
(method
  receiver: (_) @call.inner)
(method
  args: (args) @parameter.outer)

; Comments
[
  (line_comment)
  (block_comment)
] @comment.outer

; Conditionals
(ifElse) @conditional.outer
(ifElse
  condition: (_) @conditional.inner)

; Match expressions
(match) @conditional.outer
(match
  expression: (_) @conditional.inner)

; Loops (for comprehensions)
(input) @loop.outer
(input
  receipts: (receipts) @loop.inner)

; Classes/modules (contracts serve this role)
(contract) @class.outer
(contract
  name: (_) @class.inner)

; Assignments (let bindings)
(let) @assignment.outer
(let
  decls: (_) @assignment.inner)

; Returns/sends
(send) @return.outer
(send
  inputs: (_) @return.inner)

; Case branches
(case) @branch.outer
(case
  proc: (_) @branch.inner)

; Select branches
(branch) @branch.outer
(branch
  proc: (_) @branch.inner)

; New name declarations
(new) @scope.outer
(new
  decls: (name_decls) @scope.inner)

; Collections
[
  (list)
  (tuple)
  (set)
  (map)
  ; (pathmap)  ; Not in grammar 1.1.9
] @collection.outer

; Key-value pairs in maps
(key_value_pair) @entry.outer
(key_value_pair
  key: (_) @entry.key
  value: (_) @entry.value)
