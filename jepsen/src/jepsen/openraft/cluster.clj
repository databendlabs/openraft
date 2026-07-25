(ns jepsen.openraft.cluster
  (:require [clojure.tools.logging :refer [info]]
            [jepsen.core :as jepsen]
            [jepsen.openraft.client :as client]
            [jepsen.util :as util]))

(defn node-id [test node]
  (let [index (.indexOf (:nodes test) node)]
    (when (neg? index)
      (throw (ex-info "Node is not part of the test"
                      {:node node
                       :nodes (:nodes test)})))
    (inc index)))

(defn node-info [test node]
  {:node-id (node-id test node)
   :api-addr (client/api-endpoint test node)
   :raft-addr (client/raft-addr test node)})

(defn- ready-state? [state]
  (#{"Leader" "Follower"} state))

(defn- cluster-ready? [test]
  (let [leader-id (node-id test (jepsen/primary test))
        metrics (into {}
                      (map (fn [node]
                             [node (client/metrics!
                                     (client/api-endpoint test node))])
                           (:nodes test)))]
    (when (every? (fn [[_ m]]
                    (and (= leader-id (:current_leader m))
                         (ready-state? (:state m))))
                  metrics)
      metrics)))

(defn await-ready! [test]
  (util/await-fn
    #(or (cluster-ready? test)
         (throw (ex-info "OpenRaft cluster is not ready yet" {})))
    {:log-message "Waiting for OpenRaft cluster to elect a leader"
     :timeout 60000}))

(defn bootstrap! [test]
  (let [leader (jepsen/primary test)
        leader-id (node-id test leader)
        leader-endpoint (client/api-endpoint test leader)
        learners (remove #{leader} (:nodes test))]
    (info "Initializing OpenRaft cluster on" leader)
    (client/init! leader-endpoint)

    (util/await-fn
      #(let [metrics (client/metrics! leader-endpoint)]
         (if (= leader-id (:current_leader metrics))
           metrics
           (throw (ex-info "Initial OpenRaft leader is not ready yet"
                           {:metrics metrics}))))
      {:log-message "Waiting for initial OpenRaft leader"
       :timeout 60000})

    (doseq [node learners
            :let [{:keys [node-id api-addr raft-addr]} (node-info test node)]]
      (info "Adding OpenRaft learner" node)
      (client/add-learner! leader-endpoint node-id api-addr raft-addr))

    (info "Changing OpenRaft membership to" (:nodes test))
    (client/change-membership! leader-endpoint
                               (mapv #(node-id test %) (:nodes test)))

    (await-ready! test)))
