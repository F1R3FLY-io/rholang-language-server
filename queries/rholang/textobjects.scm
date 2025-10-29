; Minimal textobjects.scm for Rholang

; Functions (contracts)
(contract) @function.outer

; Blocks
(block) @block.outer

; Comments
[
  (line_comment)
  (block_comment)
] @comment.outer

; Conditionals
(ifElse) @conditional.outer

; Match expressions
(match) @conditional.outer
