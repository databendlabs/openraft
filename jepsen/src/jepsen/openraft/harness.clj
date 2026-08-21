(ns jepsen.openraft.harness
  (:require [jepsen.openraft.interruption :as interruption]))

(defn failure-state
  "Creates run-scoped state for Harness failures."
  []
  (atom {:primary nil
         :secondary []}))

(defn primary-failure
  "Returns the first recorded Harness failure, or nil."
  [state]
  (:primary @state))

(defn secondary-failures
  "Returns Harness failures recorded after the primary failure."
  [state]
  (:secondary @state))

(defn failure-snapshot
  "Returns an immutable snapshot of all recorded Harness failures."
  [state]
  @state)

(defn record-failure!
  "Atomically records a non-interruption Harness failure.

  Returns true only when this call records the primary failure. Later failures
  are retained as secondary diagnostic evidence."
  [state source context throwable]
  (if (interruption/interruption? throwable)
    false
    (let [failure {:source source
                   :context context
                   :throwable throwable}
          [before _]
          (swap-vals! state
                      (fn [{:keys [primary] :as failures}]
                        (if primary
                          (update failures :secondary conj failure)
                          (assoc failures :primary failure))))]
      (nil? (:primary before)))))
