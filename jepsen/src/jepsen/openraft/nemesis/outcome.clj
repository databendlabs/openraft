(ns jepsen.openraft.nemesis.outcome)

(defn target-category [node-count target-count]
  (cond
    (= 1 target-count) :one
    (= node-count target-count) :all
    (<= target-count (quot (dec node-count) 2)) :minority
    :else :majority))

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
