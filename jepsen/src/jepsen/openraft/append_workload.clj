(ns jepsen.openraft.append-workload
  "Elle's list-append workload: single-operation transactions that each
  append to, or read, one list.

  Every appended element is unique, so an entry applied twice shows as an
  element read twice, and a lost or reordered one as reads that disagree on a
  list's prefix. The register workload cannot show either: a register keeps
  only its last value, a replayed `Set` writes the value it already holds,
  and a replayed CAS fails its version check."
  (:require [jepsen [checker :as checker]
             [client :as client]
             [core :as jepsen]
             [generator :as gen]]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.workload :as workload]
            [jepsen.tests.cycle.append :as append]))

(def ^:private elle-options
  {:key-count 5
   :min-txn-length 1
   :max-txn-length 1
   :max-writes-per-key 128
   ;; With one operation per transaction and real-time order, strict
   ;; serializability is linearizability of each list.
   :consistency-models [:strict-serializable]})

(defn- list-key [k]
  (str "jepsen-list-" k))

(defrecord AppendClient [leader-endpoint endpoints appended-keys]
  client/Client
  (open! [this test _node]
    (assoc this
           :leader-endpoint (atom (http/api-endpoint test
                                                     (jepsen/primary test)))
           :endpoints (mapv #(http/api-endpoint test %) (:nodes test))))

  (setup! [this _test]
    this)

  (invoke! [_ _test op]
    (try
      (let [[[f k v] & more] (:value op)]
        (when (seq more)
          (throw (ex-info "The list-append client runs one operation per transaction"
                          {:value (:value op)})))
        (case f
          :append
          (do
            (swap! appended-keys conj k)
            (http/with-leader! leader-endpoint
              endpoints
              #(http/append! % (list-key k) (str v)))
            (assoc op :type :ok))

          :r
          (let [values (http/with-leader! leader-endpoint
                         endpoints
                         #(http/linearizable-list-read!
                           %
                           (list-key k))
                         http/retryable-read-error?)]
            (assoc op
                   :type :ok
                   :value [[:r k (mapv parse-long values)]]))))
      ;; Unknown exceptions, such as the one above, are rethrown as Harness
      ;; failures.
      (catch Exception e
        (workload/handle-operation-exception op e))))

  (teardown! [_ _test])

  (close! [_ _test]))

(defn- micro-op-f [op]
  (ffirst (:value op)))

(defn- meaningful-operations-checker
  "Requires an acknowledged append and a successful read in the main phase."
  []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [counts (->> history
                        (filter #(and (= :main (:phase %))
                                      (= :txn (:f %))
                                      (#{:ok :fail :info} (:type %))))
                        (map (juxt micro-op-f :type))
                        frequencies)
            missing-ok (vec (remove #(pos? (get counts [% :ok] 0))
                                    [:append :r]))]
        {:valid? (if (seq missing-ok) :unknown true)
         :missing-ok missing-ok
         :counts counts}))))

(defn workload [_opts]
  (let [appended-keys (atom (sorted-set))]
    {:client (client/validate (AppendClient. nil nil appended-keys))
     :generator (gen/clients
                 (gen/stagger 0.1
                              (gen/map #(assoc % :phase :main)
                                       (append/gen elle-options))))
     ;; Read every list once more after recovery, so an append lost in a
     ;; fault still has a later read that misses it.
     :final-generator (gen/clients
                       (delay
                         (map (fn [k]
                                {:type :invoke
                                 :f :txn
                                 :value [[:r k nil]]
                                 :phase :final
                                 :final? true})
                              @appended-keys)))
     :checker (openraft-checker/reject-checker-exceptions
               (checker/compose
                {:elle (openraft-checker/reject-checker-exceptions
                        (append/checker elle-options))
                 :meaningful-operations
                 (openraft-checker/reject-checker-exceptions
                  (meaningful-operations-checker))
                 :final-workload
                 (openraft-checker/reject-checker-exceptions
                  (workload/final-workload-checker))
                 :unexpected-sut-responses
                 (openraft-checker/reject-checker-exceptions
                  (workload/unexpected-sut-responses-checker))}))}))
