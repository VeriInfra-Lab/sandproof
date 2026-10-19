import SandboxModel.Boundaries.OwnedMessage.SharedLease

/-!
FIFO of enqueue/dequeue observations, not a simulator of Tokio's semaphore.
Enqueue here means an original send already granted by A6. In particular an
in-flight grant may enqueue after receiver close/drop; no rule says closing
instantly destroys queued or in-flight objects. Removal classifies a receive
or discard; actual payload destruction is a separate Ownership/RefStep history.
OneShot uses the same queue laws with at most one send plus the rules below.
-/
namespace SandboxModel.Boundaries.OwnedMessage

structure Buffer (α : Type) where
  enqueued : List α
  removed : List α
  queued : List α
  deriving Repr

def Fifo (s : Buffer α) : Prop := s.enqueued = s.removed ++ s.queued

inductive BufferStep : Buffer α → Buffer α → Prop
  | enqueue (e r q) (value : α) :
      BufferStep ⟨e, r, q⟩ ⟨e ++ [value], r, q ++ [value]⟩
  | receive (e r q) (value : α) :
      BufferStep ⟨e, r, value :: q⟩ ⟨e, r ++ [value], q⟩
  | discard (e r q) (value : α) :
      BufferStep ⟨e, r, value :: q⟩ ⟨e, r ++ [value], q⟩
  | pendingOrClose (s : Buffer α) : BufferStep s s

inductive BufferHistory : Buffer α → Buffer α → Prop
  | refl (s) : BufferHistory s s
  | next : BufferHistory s t → BufferStep t u → BufferHistory s u

theorem empty_fifo : Fifo (⟨[], [], []⟩ : Buffer α) := rfl

theorem buffer_step_fifo {s t : Buffer α} (step : BufferStep s t) (fifo : Fifo s) : Fifo t := by
  cases step <;> simp_all [Fifo, List.append_assoc]

theorem buffer_history_fifo {s t : Buffer α} (history : BufferHistory s t) (fifo : Fifo s) : Fifo t := by
  induction history with
  | refl => exact fifo
  | next _ step ih => exact buffer_step_fifo step ih

theorem no_fabricated_removal {s : Buffer α} (fifo : Fifo s) :
    s.removed.length + s.queued.length = s.enqueued.length := by
  rw [fifo]
  simp

theorem every_removed_value_was_enqueued {s : Buffer α} (fifo : Fifo s)
    {value : α} (removed : value ∈ s.removed) : value ∈ s.enqueued := by
  rw [fifo]
  exact List.mem_append_left _ removed

theorem fifo_receive_is_original_head (e r q : List α) (value : α)
    (fifo : Fifo ⟨e, r, value :: q⟩) : e = r ++ value :: q := fifo

theorem closed_receiver_can_retain_queue (e r q : List α) :
    BufferStep ⟨e, r, q⟩ ⟨e, r, q⟩ := .pendingOrClose _

inductive Completion (α : Type) | value (payload : α) | transportClosed
  deriving DecidableEq, Repr

structure OneShot (α : Type) where
  sender : Bool
  receiverOpen : Bool
  slot : Option α
  outcome : Option (Completion α)
  returned : Option α
  deriving Repr

-- The borrowed receiver persists when only its waiting future is cancelled.
-- Close preserves an already sent value. Receiver drop records transport loss,
-- NOT a successful receive. The sender's rejected original value is retained
-- separately from any consumer-side completion observation.
inductive OneStep : OneShot α → OneShot α → Prop
  | send (value : α) : OneStep
      ⟨true, true, none, none, none⟩ ⟨false, true, some value, none, none⟩
  | reject (value : α) : OneStep
      ⟨true, false, none, none, none⟩ ⟨false, false, none, none, some value⟩
  | senderDrop (receiver) : OneStep
      ⟨true, receiver, none, none, none⟩ ⟨false, receiver, none, none, none⟩
  | receive (value : α) (receiver returned) : OneStep
      ⟨false, receiver, some value, none, returned⟩ ⟨false, receiver, none, some (.value value), returned⟩
  | receiveClosed (receiver returned) : OneStep
      ⟨false, receiver, none, none, returned⟩ ⟨false, receiver, none, some .transportClosed, returned⟩
  | close (sender slot returned) : OneStep
      ⟨sender, true, slot, none, returned⟩ ⟨sender, false, slot, none, returned⟩
  | discard (sender receiver slot returned) : OneStep
      ⟨sender, receiver, slot, none, returned⟩ ⟨sender, false, none, some .transportClosed, returned⟩
  | cancelBorrowedWait (sender receiver slot returned) : OneStep
      ⟨sender, receiver, slot, none, returned⟩ ⟨sender, receiver, slot, none, returned⟩

-- No outcome is interpreted as CommandExit/Unavailable/End here.
theorem oneshot_cancel_keeps_slot (sender receiver) (slot returned : Option α) :
    OneStep ⟨sender, receiver, slot, none, returned⟩ ⟨sender, receiver, slot, none, returned⟩ :=
  .cancelBorrowedWait ..

theorem oneshot_close_keeps_sent_value (value : α) :
    OneStep ⟨false, true, some value, none, none⟩ ⟨false, false, some value, none, none⟩ := .close ..

theorem oneshot_sent_value_survives_close (value : α) :
    OneStep ⟨false, false, some value, none, none⟩
      ⟨false, false, none, some (.value value), none⟩ := .receive ..

theorem oneshot_rejection_returns_original (value : α) :
    OneStep ⟨true, false, none, none, none⟩ ⟨false, false, none, none, some value⟩ := .reject value

theorem oneshot_sender_loss_is_transport_error (receiver) :
    OneStep (⟨false, receiver, none, none, none⟩ : OneShot α)
      ⟨false, receiver, none, some .transportClosed, none⟩ := .receiveClosed receiver none

theorem transport_error_is_not_business_value (value : α) :
    Completion.transportClosed ≠ Completion.value value := by simp

theorem settled_oneshot_cannot_deliver_again {s t : OneShot α} (done : s.outcome.isSome = true) :
    ¬ OneStep s t := by
  intro step
  cases step <;> simp_all

theorem send_success_does_not_imply_delivery (value : α) :
    OneStep ⟨false, true, some value, none, none⟩
      ⟨false, false, none, some .transportClosed, none⟩ := .discard ..

end SandboxModel.Boundaries.OwnedMessage
