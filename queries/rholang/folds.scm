; Folds - regions that can be folded in editors

; Blocks
(block) @fold

; Collections
[
  (list)
  (tuple)
  (set)
  (map)
  ; (pathmap)  ; Not in grammar 1.1.9
] @fold

; Control structures
[
  (contract)
  (input)
  (new)
  (ifElse)
  (let)
  (bundle)
  (match)
  (choice)
] @fold

; Comments
(block_comment) @fold

; Case branches in match
(match
  (cases) @fold)

; Branch list in select
(choice
  (branches) @fold)

; Receipts in for comprehension
(input
  (receipts) @fold)

; Function-like structures with parameters
(contract
  formals: (names)? @fold)

; Send with multiple inputs
(send
  inputs: (inputs) @fold)

(send_sync
  inputs: (messages) @fold)

; Method calls with arguments
(method
  args: (args) @fold)
