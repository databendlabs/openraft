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

(defn unrecognized-installs
  "Counts values that claim an installed fault the checker cannot read.

  A coverage checker recognizes an installed outcome by the evidence it
  carries, such as the mode or the target role. A value that says `:installed`
  and fails `recognized?` is a Nemesis defect, not a fault the random schedule
  never reached, so a composed run must reject it rather than report it as
  missing coverage."
  [recognized? values]
  (->> values
       (filter #(= :installed (:status %)))
       (remove recognized?)
       count))

(defn indeterminate
  "Returns an attempted action whose side effect cannot be confirmed."
  [reason details]
  (assoc details
         :status :indeterminate
         :reason reason))
