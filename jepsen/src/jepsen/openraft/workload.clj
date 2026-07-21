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

(defn- cas-op [latest-value value-counter]
  (fn [_test _process]
    {:type :invoke
     :f :cas
     :value [@latest-value (next-value! value-counter)]}))

;; KVClient is the Jepsen client. Jepsen workers call invoke! with logical
;; operations from the generator, and this record translates them into OpenRaft
;; KV HTTP requests through jepsen.openraft.client.
(defrecord KVClient [node endpoint latest-value]
  client/Client
  (open! [this test node]
    (assoc this
           :node node
           :endpoint (http/api-endpoint test (jepsen/primary test))))

  (setup! [this _test]
    this)

  (invoke! [_ _test op]
    (case (:f op)
      :write
      (let [value (->> (:value op)
                       (http/write! endpoint key-name)
                       (remember-latest! latest-value))]
        (assoc op
               :type :ok
               :value value))

      :read
      (let [value (->> (http/linearizable-read! endpoint key-name)
                       (remember-latest! latest-value))]
        (assoc op
               :type :ok
               :value value))

      :cas
      (let [[expected new-value] (:value op)
            value (->> (http/cas! endpoint
                                  key-name
                                  (:version expected)
                                  new-value)
                       (remember-latest! latest-value))]
        (if value
          (assoc op
                 :type :ok
                 :value [expected value])
          (assoc op :type :fail)))))

  (teardown! [_ _test])

  (close! [_ _test]))

(defn workload [opts]
  (let [latest-value (atom nil)
        value-counter (atom 0)
        operations (gen/mix [read-op
                             (write-op value-counter)
                             (cas-op latest-value value-counter)])]
    {:client (client/validate (KVClient. nil nil latest-value))
     :generator (gen/clients
                  (gen/phases
                    (gen/once {:type :invoke
                               :f :write
                               :value (next-value! value-counter)})
                    (gen/time-limit (:time-limit opts)
                                    (gen/stagger 0.1 operations))
                    (gen/once read-op)))
     :checker (checker/linearizable
                {:model (model/cas-register)})}))
