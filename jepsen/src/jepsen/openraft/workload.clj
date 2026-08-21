(ns jepsen.openraft.workload
  (:require [jepsen [checker :as checker]
             [client :as client]
             [core :as jepsen]
             [generator :as gen]
             [independent :as independent]
             [random :as random]]
            [jepsen.openraft.client :as http]
            [knossos.model :as model]))

(def ^:private key-names
  (mapv #(str "jepsen-key-" %) (range 5)))

(def ^:private operation-phases
  [:bootstrap :main :final])

(def ^:private required-main-operations
  [:read :write :cas])

(def ^:private completion-types
  [:ok :fail :info])

(defn- remember-latest! [latest-values k value]
  (when value
    (swap! latest-values
           update
           k
           (fn [current]
             (if (or (nil? current)
                     (< (:version current) (:version value)))
               value
               current))))
  value)

(defn- next-value!
  "Returns a globally unique value. Uniqueness lets Knossos treat logical values
  as stable identifiers independent of server versions; see cas-op."
  [value-counter]
  (str "value-" (swap! value-counter inc)))

(defn- read-op [keys _test _process]
  (let [k (random/nth keys)]
    {:type :invoke
     :f :read
     :phase :main
     :value (independent/tuple k nil)}))

(defn- write-op [keys value-counter]
  (fn [_test _process]
    (let [k (random/nth keys)]
      {:type :invoke
       :f :write
       :phase :main
       :value (independent/tuple k (next-value! value-counter))})))

(defn- bootstrap-write-op [k value-counter]
  {:type :invoke
   :f :write
   :phase :bootstrap
   :value (independent/tuple k (next-value! value-counter))})

(defn- final-read-op [k]
  (fn [test process]
    (assoc (read-op [k] test process)
           :phase :final
           :final? true)))

(defn- final-write-op [k value-counter]
  (let [write (write-op [k] value-counter)]
    (fn [test process]
      (assoc (write test process)
             :phase :final
             :final? true))))

(defn- cas-op
  "Generates a CAS against the latest known value, falling back to a write until
  a version is observed.

  Knossos checks linearizability over the logical string values only. This is
  sound because every applied logical value identifies one server version:

    1. next-value! never repeats a value, so every mutation is globally unique.
    2. with-leader! retries a mutation only after a failure proving it was not
       applied, never after an ambiguous outcome, so each unique value is
       applied at most once.

  Server versions need not be contiguous: other keys and Raft-internal entries
  can consume log indexes. Breaking either invariant above would map one logical
  value onto two versions and could mask a real violation."
  [keys latest-values value-counter]
  (fn [_test _process]
    (let [k (random/nth keys)
          expected (get @latest-values k)
          new-value (next-value! value-counter)]
      (if expected
        {:type :invoke
         :f :cas
         :phase :main
         ;; Knossos models logical values. The version is request metadata for
         ;; OpenRaft's version-based CAS and is not part of the model value.
         :value (independent/tuple k [(:value expected) new-value])
         :expected-version (:version expected)}
        {:type :invoke
         :f :write
         :phase :main
         :value (independent/tuple k new-value)}))))

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
  Thread interruptions are rethrown so Jepsen's control signals survive."
  [op e]
  ;; A raw InterruptedException (thrown outside send!, which clears the interrupt
  ;; flag) would otherwise fall through to the :client-error default and be
  ;; swallowed. Restore the flag and rethrow so the interrupt is not lost.
  (when (instance? InterruptedException e)
    (.interrupt (Thread/currentThread))
    (throw e))
  (let [{:keys [kind status error]} (ex-data e)]
    ;; send! already re-interrupted the thread for wrapped interruptions.
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

              (throw e))]
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

(defn- empty-operation-counts []
  (into {}
        (map (fn [phase]
               [phase
                (into {}
                      (map (fn [f]
                             [f (zipmap completion-types (repeat 0))])
                           required-main-operations))])
             operation-phases)))

(defn- meaningful-operations-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [counts (reduce
                    (fn [counts {:keys [phase f type]}]
                      (if (and (some #{phase} operation-phases)
                               (some #{f} required-main-operations)
                               (some #{type} completion-types))
                        (update-in counts [phase f type] inc)
                        counts))
                    (empty-operation-counts)
                    history)
            missing-ok (->> required-main-operations
                            (filter #(zero? (get-in counts
                                                    [:main % :ok])))
                            vec)]
        {:valid? (if (seq missing-ok) :unknown true)
         :missing-ok missing-ok
         :counts counts}))))

;; KVClient is the Jepsen client. Jepsen workers call invoke! with logical
;; operations from the generator, and this record translates them into OpenRaft
;; KV HTTP requests through jepsen.openraft.client.
(defrecord KVClient [node leader-endpoint endpoints latest-values]
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
      (let [k (key (:value op))
            op-value (val (:value op))]
        (case (:f op)
          :write
          (let [value (http/with-leader!
                        leader-endpoint
                        endpoints
                        #(http/write! % k op-value))]
            (remember-latest! latest-values k value)
            (assoc op :type :ok))

          :read
          (let [value (http/with-leader!
                        leader-endpoint
                        endpoints
                        #(http/linearizable-read! % k)
                        http/retryable-read-error?)]
            (remember-latest! latest-values k value)
            (assoc op
                   :type :ok
                   :value (independent/tuple k (:value value))))

          :cas
          (let [[_expected new-value] op-value
                value (http/with-leader!
                        leader-endpoint
                        endpoints
                        #(http/cas! %
                                    k
                                    (:expected-version op)
                                    new-value))]
            (remember-latest! latest-values k value)
            (if value
              (assoc op :type :ok)
              (assoc op
                     :type :fail
                     :error :version-mismatch)))))
      (catch Exception e
        (handle-operation-exception op e))))

  (teardown! [_ _test])

  (close! [_ _test]))

(defn workload [_opts]
  (let [latest-values (atom {})
        value-counter (atom 0)
        operations (gen/mix [(partial read-op key-names)
                             (write-op key-names value-counter)
                             (cas-op key-names latest-values value-counter)])
        bootstrap (apply gen/phases
                         (map #(gen/once
                                (bootstrap-write-op % value-counter))
                              key-names))
        final (apply gen/phases
                     (concat
                      (map #(gen/once
                             (final-write-op % value-counter))
                           key-names)
                      (map #(gen/once (final-read-op %)) key-names)))]
    {:client (client/validate (KVClient. nil nil nil latest-values))
     :generator (gen/clients
                 (gen/phases
                  bootstrap
                  (gen/stagger 0.1 operations)))
     :final-generator (gen/clients final)
     :checker (checker/compose
               {:linearizable (independent/checker
                               (checker/linearizable
                                {:model (model/cas-register)}))
                :meaningful-operations (meaningful-operations-checker)
                :unexpected-errors (unexpected-errors-checker)})}))
