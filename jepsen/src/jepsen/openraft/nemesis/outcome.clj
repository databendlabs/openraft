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

(defn malformed-outcomes
  "Counts fault outcomes a coverage checker cannot read.

  A checker recognizes an installed outcome by the evidence it carries, such
  as the mode or the target role. Two shapes are defects of the Nemesis rather
  than a fault the random schedule never reached, so a run must reject them
  instead of reporting missing coverage: an `:installed` value that fails
  `recognized?`, and a value whose `:status` is none of `:installed`,
  `:skipped` and `:indeterminate`.

  A `:skipped` outcome stays legal, because relaxing skipped faults is what a
  composed run is built on, and an `:indeterminate` one keeps the lifecycle
  meaning its own checker gives it.

  A value carrying no `:status` at all is passed over. The history holds the
  invocation and the completion of every Nemesis operation under one `:f`, the
  Jepsen interpreter gives the two entries separate `:index` values, and an
  invocation's value never carries a `:status`. A checker reading the history
  therefore cannot tell an invocation apart from a completion whose status went
  missing, and rejecting a missing `:status` would reject every real run."
  [recognized? values]
  (->> values
       (filter #(and (map? %) (contains? % :status)))
       (remove (fn [value]
                 (case (:status value)
                   :installed (boolean (recognized? value))
                   (:skipped :indeterminate) true
                   false)))
       count))

(defn indeterminate
  "Returns an attempted action whose side effect cannot be confirmed."
  [reason details]
  (assoc details
         :status :indeterminate
         :reason reason))
