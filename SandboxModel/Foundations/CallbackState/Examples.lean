import SandboxModel.Foundations.CallbackState.Instances

namespace SandboxModel.Foundations.CallbackState
open Aeneas Aeneas.Std
open SandboxModel.Foundations

theorem toggle_duplicate_positions_example :
    tableRetainState toggleModel false [(7#u64, 11#u64), (7#u64, 12#u64), (7#u64, 11#u64)] =
      ([(7#u64, 11#u64), (7#u64, 11#u64)], true) := by rfl

theorem trace_complete_values_and_duplicates_example :
    tableRetainState (traceModel 11#u64) [(8#u64, 9#u64)]
        [(7#u64, 11#u64), (7#u64, 12#u64), (7#u64, 11#u64)] =
      ([(7#u64, 12#u64)],
       [(8#u64, 9#u64), (7#u64, 11#u64), (7#u64, 12#u64), (7#u64, 11#u64)]) := by rfl

theorem empty_toggle_example (state : Bool) :
    tableRetainState toggleModel state ([] : List Row) = ([], state) := by rfl

end SandboxModel.Foundations.CallbackState
