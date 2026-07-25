(ns jepsen.openraft.cluster
  (:require [clojure.tools.logging :refer [info]]
            [jepsen.core :as jepsen]
            [jepsen.openraft.client :as client]
            [jepsen.util :as util]))

;; TODO: Use an explicit node-name to OpenRaft ID mapping before introducing
;; membership change nemeses.
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

(defn- cluster-status [test]
  (let [metrics (into {}
                      (map (fn [node]
                             [node (client/metrics!
                                     (client/api-endpoint test node))])
                           (:nodes test)))
        leader-ids (set (map :current_leader (vals metrics)))
        leaders (filter (fn [[_ metrics]]
                          (= "Leader" (:state metrics)))
                        metrics)]
    (when (and (= 1 (count leader-ids))
               (= 1 (count leaders))
               (every? (comp ready-state? :state val) metrics))
      (let [leader-id (first leader-ids)
            [leader _] (first leaders)]
        (when (= leader-id (node-id test leader))
          {:leader leader
           :metrics metrics})))))

(defn await-ready! [test]
  (util/await-fn
    #(or (cluster-status test)
         (throw (ex-info "OpenRaft cluster is not ready yet" {})))
    {:log-message "Waiting for every OpenRaft node to agree on a leader"
     :timeout 60000}))

(defn voter-configs
  "Maps the effective OpenRaft voter configs to Jepsen node names."
  [test {:keys [leader metrics]}]
  (let [configs (get-in metrics
                        [leader
                         :membership_config
                         :membership
                         :configs])
        voter-ids (set (mapcat identity configs))]
    (when-not (seq configs)
      (throw (ex-info "OpenRaft metrics contain no voter configs"
                      {:leader leader
                       :metrics (get metrics leader)})))
    (let [nodes-by-id (into {}
                            (map (fn [node]
                                   [(node-id test node) node])
                                 (:nodes test)))
          unknown-ids (remove #(contains? nodes-by-id %) voter-ids)]
      (when (seq unknown-ids)
        (throw (ex-info "OpenRaft voter is not a Jepsen test node"
                        {:voter-ids voter-ids
                         :unknown-ids (vec unknown-ids)})))
      (mapv (fn [config]
              (set (map nodes-by-id config)))
            configs))))

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
