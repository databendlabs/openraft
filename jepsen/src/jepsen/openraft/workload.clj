(ns jepsen.openraft.workload
  (:require [jepsen [checker :as checker]
                    [client :as client]
                    [core :as jepsen]
                    [generator :as gen]]
            [jepsen.openraft.client :as http]
            [knossos.model :as model]))

(def key-name "jepsen-key")

(defn- remember-latest! [latest-value value]
  (when value
    (swap! latest-value
           (fn [current]
             (if (or (nil? current)
                     (< (:version current) (:version value)))
               value
               current))))
  value)

(defn- next-value! [value-counter]
  (str "value-" (swap! value-counter inc)))

(defn- read-op [_test _process]
  {:type :invoke
   :f :read
   :value nil})

(defn- write-op [value-counter]
  (fn [_test _process]
    {:type :invoke
     :f :write
     :value (next-value! value-counter)}))

(defn- final-read-op [test process]
  (assoc (read-op test process) :final? true))

(defn- final-write-op [value-counter]
  (let [write (write-op value-counter)]
    (fn [test process]
      (assoc (write test process) :final? true))))

(defn- cas-op [latest-value value-counter]
  (fn [_test _process]
    (let [expected @latest-value
          new-value (next-value! value-counter)]
      (if expected
        {:type :invoke
         :f :cas
         ;; Knossos models logical values. The version is request metadata for
         ;; OpenRaft's version-based CAS and is not part of the model value.
         :value [(:value expected) new-value]
         :expected-version (:version expected)}
        {:type :invoke
         :f :write
         :value new-value}))))

(defn- mutation? [op]
  (#{:write :cas} (:f op)))

(defn- complete-with-error [op type error unexpected?]
  (cond-> (assoc op
                 :type type
                 :error error)
    unexpected? (assoc :unexpected? true)))

(defn- complete-with-ambiguous-outcome
  "Marks an ambiguous mutation as info and a read as failed."
  [op error unexpected?]
  (complete-with-error op
                       (if (mutation? op) :info :fail)
                       error
                       unexpected?))

(defn- handle-operation-exception
  "Classifies a client exception and returns a completed Jepsen operation.
  Thread interruptions are rethrown."
  [op e]
  (let [{:keys [kind status error]} (ex-data e)]
    (if (= :interrupted kind)
      (throw e)
      (let [result
            (case kind
              :unreachable
              (complete-with-error op :fail :unreachable false)

              :request-timeout
              (complete-with-ambiguous-outcome op :timeout false)

              :transport-error
              (complete-with-ambiguous-outcome op :transport-error false)

              :http-error
              ;; app-http returns 4xx before invoking a handler, while a 5xx
              ;; may occur while serializing a response after a mutation has
              ;; completed.
              (if (and status (<= 400 status 499))
                (complete-with-error op :fail [:http status] true)
                (complete-with-ambiguous-outcome op [:http status] true))

              :invalid-json
              (complete-with-ambiguous-outcome op :invalid-json true)

              :openraft-error
              (cond
                (contains? error :ForwardToLeader)
                (complete-with-error op :fail :not-leader false)

                (contains? error :QuorumNotEnough)
                (complete-with-error op :fail :quorum-not-enough false)

                :else
                (complete-with-ambiguous-outcome op :openraft-error true))

              (complete-with-ambiguous-outcome op :client-error true))]
        (cond-> result
          (or (:unexpected? result) (:final? op))
          (assoc :exception-message (ex-message e)
                 :exception-data (ex-data e)
                 :unexpected? true))))))

(defn- unexpected-errors-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [errors (filter :unexpected? history)]
        {:valid? (empty? errors)
         :count (count errors)
         :errors (vec (take 10 errors))}))))

;; KVClient is the Jepsen client. Jepsen workers call invoke! with logical
;; operations from the generator, and this record translates them into OpenRaft
;; KV HTTP requests through jepsen.openraft.client.
(defrecord KVClient [node leader-endpoint endpoints latest-value]
  client/Client
  (open! [this test node]
    (assoc this
           :node node
           :leader-endpoint (atom (http/api-endpoint test
                                                     (jepsen/primary test)))
           :endpoints (mapv #(http/api-endpoint test %) (:nodes test))))

  (setup! [this _test]
    this)

  (invoke! [_ _test op]
    (try
      (case (:f op)
        :write
        (let [value (http/with-leader!
                      leader-endpoint
                      endpoints
                      #(http/write! % key-name (:value op)))]
          (remember-latest! latest-value value)
          (assoc op :type :ok))

        :read
        (let [value (->> (http/with-leader!
                           leader-endpoint
                           endpoints
                           #(http/linearizable-read! % key-name)
                           http/retryable-read-error?)
                         (remember-latest! latest-value))]
          (assoc op
                 :type :ok
                 :value (:value value)))

        :cas
        (let [[_expected new-value] (:value op)
              value (->> (http/with-leader!
                           leader-endpoint
                           endpoints
                           #(http/cas! %
                                       key-name
                                       (:expected-version op)
                                       new-value))
                         (remember-latest! latest-value))]
          (if value
            (assoc op :type :ok)
            (assoc op
                   :type :fail
                   :error :version-mismatch))))
      (catch Exception e
        (handle-operation-exception op e))))

  (teardown! [_ _test])

  (close! [_ _test]))

(defn workload [_opts]
  (let [latest-value (atom nil)
        value-counter (atom 0)
        operations (gen/mix [read-op
                             (write-op value-counter)
                             (cas-op latest-value value-counter)])]
    {:client (client/validate (KVClient. nil nil nil latest-value))
     :generator (gen/clients
                  (gen/phases
                    (gen/once {:type :invoke
                               :f :write
                               :value (next-value! value-counter)})
                    (gen/stagger 0.1 operations)))
     :final-generator (gen/clients
                        (gen/phases
                          (gen/once (final-write-op value-counter))
                          (gen/once final-read-op)))
     :checker (checker/compose
                {:linearizable (checker/linearizable
                                 {:model (model/cas-register)})
                 :unexpected-errors (unexpected-errors-checker)})}))
