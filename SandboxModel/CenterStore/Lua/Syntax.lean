import Std

/-! Restricted, terminating syntax for the two frozen ASCII Redis inputs.
Unsupported constructs fail; this module does not assert full Lua semantics. -/

open Lean in
macro "luaChars%" s:str : term => do
  let values := s.getString.toList
  if values.length ≤ 64 then return quote values
  let mut rest := values
  let mut chunks : Array (TSyntax `term) := #[]
  while !rest.isEmpty do
    chunks := chunks.push (quote (rest.take 64))
    rest := rest.drop 64
  `(List.flatten [$[$chunks],*])

namespace Sandproof.CenterStore.Lua

inductive Token where
  | word (value : List Char)
  | quoted (value : List Char)
  | number (value : Nat)
  | symbol (value : List Char)
  deriving DecidableEq, Repr

private def identifierStart (c : Char) : Bool :=
  ('a' ≤ c && c ≤ 'z') || ('A' ≤ c && c ≤ 'Z') || c == '_'

private def identifierPart (c : Char) : Bool := identifierStart c || c.isDigit

private def lexAux : Nat → List Char → Option (List Token)
  | 0, _ => none
  | _ + 1, [] => some []
  | fuel + 1, c :: cs => do
    if c.isWhitespace then return ← lexAux fuel cs
    if c == '-' && cs.head? == some '-' then
      if (cs.drop 1).head? == some '[' then none
      else return ← lexAux fuel (cs.dropWhile (· != '\n'))
    if c == '\'' || c == '"' then
      let (body, rest) := cs.span (· != c)
      if body.any (fun d => d == '\\' || d == '\n' || d == '\r') then none
      else match rest with
      | [] => none
      | _ :: tail => return .quoted (body) :: (← lexAux fuel tail)
    else if identifierStart c then
      let (tail, rest) := cs.span identifierPart
      return .word ((c :: tail)) :: (← lexAux fuel rest)
    else if c.isDigit then
      let (tail, rest) := cs.span Char.isDigit
      let n := (c :: tail).foldl (fun acc digit => acc * 10 + (digit.toNat - 48)) 0
      return .number n :: (← lexAux fuel rest)
    else
      let pair := (c :: cs.take 1)
      if pair == (luaChars% "==") || pair == (luaChars% "~=") || pair == (luaChars% "<=") || pair == (luaChars% ">=") then
        return .symbol pair :: (← lexAux fuel (cs.drop 1))
      else if (luaChars% "(){}[],.=<>-").contains c then
        return .symbol [c] :: (← lexAux fuel cs)
      else none

def lex (source : String) : Option (List Token) :=
  lexAux (source.length + 1) source.toList

inductive BinOp where
  | or | and | eq | ne | lt | le | gt | ge
  deriving DecidableEq, Repr

inductive Expr where
  | text (value : List Char)
  | number (value : Int)
  | variable (name : List Char)
  | field (object : Expr) (name : List Char)
  | index (object : Expr) (index : Nat)
  | call (function : Expr) (arguments : List Expr)
  | array (values : List Expr)
  | not (value : Expr)
  | neg (value : Expr)
  | binary (op : BinOp) (left right : Expr)
  deriving Repr

private def binaryOp : Token → Option (BinOp × Nat)
  | .word (luaChars% "or") => some (.or, 1)
  | .word (luaChars% "and") => some (.and, 2)
  | .symbol (luaChars% "==") => some (.eq, 3)
  | .symbol (luaChars% "~=") => some (.ne, 3)
  | .symbol (luaChars% "<") => some (.lt, 3)
  | .symbol (luaChars% "<=") => some (.le, 3)
  | .symbol (luaChars% ">") => some (.gt, 3)
  | .symbol (luaChars% ">=") => some (.ge, 3)
  | _ => none

private def reserved (s : List Char) : Bool :=
  [(luaChars% "and"), (luaChars% "break"), (luaChars% "do"), (luaChars% "else"), (luaChars% "elseif"), (luaChars% "end"), (luaChars% "false"), (luaChars% "for"), (luaChars% "function"),
   (luaChars% "if"), (luaChars% "in"), (luaChars% "local"), (luaChars% "nil"), (luaChars% "not"), (luaChars% "or"), (luaChars% "repeat"), (luaChars% "return"), (luaChars% "then"), (luaChars% "true"), (luaChars% "until"),
   (luaChars% "while")].contains s

mutual
  private def parseExpr : Nat → Nat → List Token → Option (Expr × List Token)
    | 0, _, _ => none
    | fuel + 1, precedence, tokens => do
      let (first, rest) ← match tokens with
        | .word (luaChars% "not") :: tail => do
          let (e, rest) ← parseExpr fuel 4 tail
          pure (.not e, rest)
        | .symbol (luaChars% "-") :: tail => do
          let (e, rest) ← parseExpr fuel 4 tail
          pure (.neg e, rest)
        | .quoted s :: tail => parseSuffix fuel (.text s) tail
        | .number n :: tail => parseSuffix fuel (.number n) tail
        | .word s :: tail =>
          if reserved s then none else parseSuffix fuel (.variable s) tail
        | .symbol (luaChars% "(") :: tail => do
          let (e, .symbol (luaChars% ")") :: rest) ← parseExpr fuel 0 tail | none
          parseSuffix fuel e rest
        | .symbol (luaChars% "{") :: tail => do
          let (es, rest) ← parseArguments fuel (luaChars% "}") tail
          parseSuffix fuel (.array es) rest
        | _ => none
      parseInfix fuel precedence first rest

  termination_by structural fuel _ _ => fuel

  private def parseSuffix : Nat → Expr → List Token → Option (Expr × List Token)
    | 0, _, _ => none
    | fuel + 1, e, .symbol (luaChars% ".") :: .word field :: rest =>
      parseSuffix fuel (.field e field) rest
    | fuel + 1, e, .symbol (luaChars% "[") :: .number n :: .symbol (luaChars% "]") :: rest =>
      parseSuffix fuel (.index e n) rest
    | fuel + 1, e, .symbol (luaChars% "(") :: rest => do
      let (args, rest) ← parseArguments fuel (luaChars% ")") rest
      parseSuffix fuel (.call e args) rest
    | _ + 1, e, rest => some (e, rest)

  termination_by structural fuel _ _ => fuel

  private def parseArguments : Nat → List Char → List Token → Option (List Expr × List Token)
    | 0, _, _ => none
    | fuel + 1, closing, tokens => do
      if tokens.head? == some (.symbol closing) then return ([], tokens.drop 1)
      let (e, rest) ← parseExpr fuel 0 tokens
      match rest with
      | .symbol (luaChars% ",") :: tail =>
        if tail.head? == some (.symbol closing) then none
        else do
          let (es, rest) ← parseArguments fuel closing tail
          pure (e :: es, rest)
      | .symbol s :: tail => if s == closing then some ([e], tail) else none
      | _ => none

  termination_by structural fuel _ _ => fuel

  private def parseInfix : Nat → Nat → Expr → List Token → Option (Expr × List Token)
    | 0, _, _, _ => none
    | _ + 1, _, left, [] => some (left, [])
    | fuel + 1, precedence, left, token :: rest => do
      match binaryOp token with
      | none => some (left, token :: rest)
      | some (op, p) =>
        if p < precedence then some (left, token :: rest)
        else do
          let (right, rest) ← parseExpr fuel (p + 1) rest
          parseInfix fuel precedence (.binary op left right) rest
  termination_by structural fuel _ _ _ => fuel
end

inductive Stmt where
  | local (name : List Char) (value : Expr)
  | branch (condition : Expr) (yes no : List Stmt)
  | returns (value : Expr)
  | invoke (call : Expr)
  deriving Repr

private def blockEnd (tokens : List Token) : Bool :=
  tokens.isEmpty || tokens.head? == some (.word (luaChars% "end")) ||
    tokens.head? == some (.word (luaChars% "else"))

mutual
  private def parseBlock : Nat → List Token → Option (List Stmt × List Token)
    | 0, _ => none
    | fuel + 1, tokens => do
      if blockEnd tokens then return ([], tokens)
      if tokens.head? == some (.word (luaChars% "return")) then
        let (e, rest) ← parseExpr fuel 0 (tokens.drop 1)
        if blockEnd rest then some ([.returns e], rest) else none
      else
        let (stmt, rest) ← parseStatement fuel tokens
        let (tail, rest) ← parseBlock fuel rest
        pure (stmt :: tail, rest)

  termination_by structural fuel _ => fuel

  private def parseStatement : Nat → List Token → Option (Stmt × List Token)
    | 0, _ => none
    | fuel + 1, .word (luaChars% "local") :: .word name :: .symbol (luaChars% "=") :: rest => do
      if reserved name then none else do
        let (e, rest) ← parseExpr fuel 0 rest
        pure (.local name e, rest)
    | fuel + 1, .word (luaChars% "if") :: rest => do
      let (condition, .word (luaChars% "then") :: rest) ← parseExpr fuel 0 rest | none
      let (yes, rest) ← parseBlock fuel rest
      match rest with
      | .word (luaChars% "end") :: tail => some (.branch condition yes [], tail)
      | .word (luaChars% "else") :: tail => do
        let (no, .word (luaChars% "end") :: rest) ← parseBlock fuel tail | none
        pure (.branch condition yes no, rest)
      | _ => none
    | fuel + 1, tokens => do
      let (e, rest) ← parseExpr fuel 0 tokens
      match e with
      | .call _ _ => some (.invoke e, rest)
      | _ => none
  termination_by structural fuel _ => fuel
end

def parseChars (source : List Char) : Option (List Stmt) := do
  let tokens ← lexAux (source.length + 1) source
  let (statements, []) ← parseBlock 64 tokens | none
  pure statements

def parse (source : String) := parseChars source.toList

end Sandproof.CenterStore.Lua
