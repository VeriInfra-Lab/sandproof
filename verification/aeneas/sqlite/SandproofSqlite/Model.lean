import SandproofSqlite.Types
import SandproofSqlite.SlotData

open Aeneas Aeneas.Std
namespace SandproofSqliteRust.Model

def slotOps (W : Type) : controller.transaction.memo.Cells (Slots W) W where
  set_committed := fun state value => .ok { state with committed := value }
  take_candidate := fun state => .ok (state.candidate, { state with candidate := none })

end SandproofSqliteRust.Model
