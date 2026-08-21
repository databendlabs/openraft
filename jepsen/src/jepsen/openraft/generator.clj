(ns jepsen.openraft.generator
  (:require [jepsen.generator :as gen]
            [jepsen.openraft.harness :as harness]))

(defrecord StopOnHarnessFailure [failure-state generator]
  gen/Generator
  (op [_ test context]
    (when-not (harness/primary-failure failure-state)
      (when-let [[operation generator'] (gen/op generator test context)]
        (when-not (harness/primary-failure failure-state)
          [operation (StopOnHarnessFailure. failure-state generator')]))))

  (update [_ test context event]
    (when-let [generator' (gen/update generator test context event)]
      (StopOnHarnessFailure. failure-state generator'))))

(defn stop-on-harness-failure
  "Stops new operations after a Harness failure while forwarding updates."
  [failure-state generator]
  (when generator
    (StopOnHarnessFailure. failure-state generator)))

(defrecord PendingOnHarnessFailure [failure-state generator]
  gen/Generator
  (op [_ test context]
    (if (harness/primary-failure failure-state)
      [:pending (PendingOnHarnessFailure. failure-state generator)]
      (when-let [[operation generator'] (gen/op generator test context)]
        [operation (PendingOnHarnessFailure. failure-state generator')])))

  (update [_ test context event]
    (let [generator' (gen/update generator test context event)]
      (if (harness/primary-failure failure-state)
        (PendingOnHarnessFailure. failure-state generator')
        (when generator'
          (PendingOnHarnessFailure. failure-state generator'))))))

(defn pending-on-harness-failure
  "Stops scheduling work after a Harness failure without exhausting a parent
  generator that must still run recovery."
  [failure-state generator]
  (when generator
    (PendingOnHarnessFailure. failure-state generator)))
