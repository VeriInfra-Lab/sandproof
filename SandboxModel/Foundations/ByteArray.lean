import Aeneas
import Init.Data.ByteArray.Lemmas

namespace SandboxModel.Foundations

theorem byteArray_toList_loop (bs : ByteArray) (i : Nat) (acc : List UInt8) :
    ByteArray.toList.loop bs i acc = acc.reverse ++ bs.data.toList.drop i := by
  induction i, acc using ByteArray.toList.loop.induct bs with
  | case1 i acc within ih =>
    have dataWithin : i < bs.data.size := by simpa using within
    have listWithin : i < bs.data.toList.length := by simpa using within
    have sameByte : bs.get! i = bs.data.toList[i]'listWithin := by
      change bs.data[i]! = bs.data.toList[i]'listWithin
      rw [getElem!_pos bs.data i dataWithin, Array.getElem_toList]
    have sameTail :
        bs.get! i :: bs.data.toList.drop (i + 1) = bs.data.toList.drop i := by
      rw [sameByte]
      exact List.getElem_cons_drop listWithin
    rw [ByteArray.toList.loop, if_pos within, ih, List.reverse_cons,
      List.append_assoc]
    simp only [List.singleton_append, sameTail]
  | case2 i acc outside =>
    have exhausted : bs.data.toList.length ≤ i := by
      simpa using Nat.le_of_not_lt outside
    rw [ByteArray.toList.loop, if_neg outside]
    simp only [List.drop_eq_nil_of_le exhausted, List.append_nil]

theorem byteArray_toList_data (bs : ByteArray) :
    bs.toList = bs.data.toList := by
  simpa only [ByteArray.toList, List.reverse_nil, List.nil_append, List.drop_zero]
    using byteArray_toList_loop bs 0 []

end SandboxModel.Foundations
