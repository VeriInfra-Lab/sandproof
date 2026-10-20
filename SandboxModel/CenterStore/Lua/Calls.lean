import SandboxModel.CenterStore.Lua.Syntax

namespace Sandproof.CenterStore.Lua

def keyOne : Expr → Bool
  | .index (.variable (luaChars% "KEYS")) 1 => true
  | _ => false

/-- Syntactic use domain, not a theorem about unregistered function values. -/
def readOnly : Nat → Expr → Bool
  | 0, _ => false
  | _ + 1, .text _ | _ + 1, .number _ | _ + 1, .variable _ => true
  | fuel + 1, .field e _ | fuel + 1, .index e _ => readOnly fuel e
  | fuel + 1, .array es => es.all (readOnly fuel)
  | fuel + 1, .not e | fuel + 1, .neg e => readOnly fuel e
  | fuel + 1, .binary _ a b => readOnly fuel a && readOnly fuel b
  | fuel + 1, .call (.variable (luaChars% "tonumber")) [e] => readOnly fuel e
  | fuel + 1, .call (.field (.variable (luaChars% "string")) (luaChars% "len")) [e] =>
      readOnly fuel e
  | fuel + 1, .call (.field (.variable (luaChars% "redis")) (luaChars% "call"))
      (.text command :: key :: args) =>
    keyOne key && args.all (readOnly fuel) &&
      (([(luaChars% "TYPE"), (luaChars% "HLEN"), (luaChars% "PTTL")].contains command &&
        args.isEmpty) ||
       ([(luaChars% "HGET"), (luaChars% "HSTRLEN")].contains command && args.length == 1))
  | _ + 1, _ => false

def writeCall : Expr → Bool
  | .call (.field (.variable (luaChars% "redis")) (luaChars% "call"))
      (.text (luaChars% "HSET") :: key :: args) =>
    keyOne key && args.length ≥ 2 && args.length % 2 == 0 && args.all (readOnly 64)
  | _ => false

def protectedGlobal (name : List Char) : Bool :=
  [(luaChars% "KEYS"), (luaChars% "ARGV"), (luaChars% "redis"),
   (luaChars% "string"), (luaChars% "tonumber")].contains name

def callsSafe : Nat → Stmt → Bool
  | 0, _ => false
  | _ + 1, .local name e => !protectedGlobal name && readOnly 64 e
  | _ + 1, .returns e => readOnly 64 e
  | _ + 1, .invoke call => writeCall call
  | fuel + 1, .branch condition yes no =>
      readOnly 64 condition && yes.all (callsSafe fuel) && no.all (callsSafe fuel)

end Sandproof.CenterStore.Lua
