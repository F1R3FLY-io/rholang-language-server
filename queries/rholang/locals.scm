; Minimal locals.scm for Rholang

; Source file is root scope
(source_file) @local.scope

; Blocks create scopes
(block) @local.scope

; Variables are references
(var) @local.reference
