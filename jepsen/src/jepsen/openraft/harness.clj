(ns jepsen.openraft.harness
  (:require [jepsen.openraft.interruption :as interruption]))

(defn failure-state
  "Creates run-scoped state for the primary Harness failure."
  []
  (atom nil))

(defn primary-failure
  "Returns the first recorded Harness failure, or nil."
  [state]
  @state)

(defn record-failure!
  "Atomically records the first non-interruption Harness failure."
  [state source context throwable]
  (and (not (interruption/interruption? throwable))
       (compare-and-set! state
                         nil
                         {:source source
                          :context context
                          :throwable throwable})))
