(ns jepsen.openraft.workload
  (:require [jepsen [checker :as checker]
                    [client :as client]
                    [core :as jepsen]
                    [generator :as gen]]
            [jepsen.openraft.client :as http]))

(def key-name "jepsen-key")
(def value-name "jepsen-value")

;; KVClient is the Jepsen client. Jepsen workers call invoke! with logical
;; operations from the generator, and this record translates them into OpenRaft
;; KV HTTP requests through jepsen.openraft.client.
(defrecord KVClient [node endpoint]
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
      (let [[k v] (:value op)]
        (http/write! endpoint k v)
        (assoc op :type :ok))

      :read
      (assoc op
             :type :ok
             :value (http/read! endpoint (:value op)))))

  (teardown! [_ _test])

  (close! [_ _test]))

(defn workload [_opts]
  {:client (client/validate (KVClient. nil nil))
   :generator (gen/clients
                (gen/phases
                  (gen/once {:type :invoke
                             :f :write
                             :value [key-name value-name]})
                  (gen/once {:type :invoke
                             :f :read
                             :value key-name})))
   :checker (checker/unbridled-optimism)})
