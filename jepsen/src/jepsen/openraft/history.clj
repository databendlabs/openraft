(ns jepsen.openraft.history)

(defn nemesis-events
  "Returns nemesis events for operation f whose value has status."
  [history f status]
  (filter #(and (= :nemesis (:process %))
                (= f (:f %))
                (= status (get-in % [:value :status])))
          history))

(defn successful-writes
  "Returns writes invoked and completed successfully within [start, end)."
  [history start end]
  (:writes
   (reduce (fn [{:keys [pending] :as acc} op]
             (cond
               (= :invoke (:type op))
               (assoc-in acc [:pending (:process op)] op)

               (#{:ok :fail :info} (:type op))
               (let [invocation (get pending (:process op))]
                 (cond-> (update acc :pending dissoc (:process op))
                   (and invocation
                        (= :ok (:type op))
                        (= :write (:f op))
                        (<= start (:time invocation) (:time op))
                        (< (:time op) end))
                   (update :writes conj op)))

               :else acc))
           {:pending {} :writes []}
           history)))
