; Scopes - define lexical scopes in Rholang

; Source file is the root scope
(source_file) @local.scope

; Blocks create new scopes
(block) @local.scope

; New declarations create a scope for the declared names
(new
  (name_decls) @local.definition
  (proc) @local.scope)

(name_decl (var) @local.definition)

; Contracts create scopes with parameters as definitions
(contract
  name: (_) @local.definition
  formals: (names)? @local.definition
  proc: (block) @local.scope)

; For comprehensions create scopes for bound names
(input
  receipts: (receipts) @local.definition
  proc: (block) @local.scope)

; Linear binds define names
(linear_bind
  names: (names)? @local.definition)

; Repeated binds define names
(repeated_bind
  names: (names)? @local.definition)

; Peek binds define names
(peek_bind
  names: (names)? @local.definition)

; Let expressions create scopes with bindings
(let
  decls: (_) @local.definition
  proc: (_) @local.scope)

; Declarations in let
(decl
  names: (names) @local.definition)

; Match cases can bind names in patterns
(match
  cases: (cases
    (case
      pattern: (_) @local.definition
      proc: (_) @local.scope)))

; Select branches bind names
(choice
  branches: (branches
    (branch
      pattern: (_) @local.definition
      proc: (_) @local.scope)))

; Variable references
(var) @local.reference
(var_ref var: (var) @local.reference)

; Eval references a name
(eval name: (_) @local.reference)
