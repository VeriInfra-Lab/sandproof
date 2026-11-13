import SandproofFoundations.FunsExternal
import SandproofSourceLive.Types
open Aeneas Aeneas.Std
namespace SandproofSourceLiveRust

def core.option.Option.is_some_and {T F : Type}
    (callback : core.ops.function.FnOnce F T Bool) (value : Option T) (state : F) : Result Bool :=
  SandproofFoundationsRust.core.option.Option.is_some_and callback value state

end SandproofSourceLiveRust
