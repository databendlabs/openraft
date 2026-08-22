(ns jepsen.openraft.nemesis.outcome)

(defn installed
  "Returns a confirmed Nemesis outcome with structured details."
  [details]
  (assoc details :status :installed))

(defn skipped
  "Returns a known non-installation with a structured reason."
  [reason details]
  (assoc details
         :status :skipped
         :reason reason))

(defn indeterminate
  "Returns an attempted action whose side effect cannot be confirmed."
  [reason details]
  (assoc details
         :status :indeterminate
         :reason reason))
