import Aeneas

namespace SandproofSqliteRust.Model

structure Slots (W : Type) where
  committed : Option W
  candidate : Option W

end SandproofSqliteRust.Model
