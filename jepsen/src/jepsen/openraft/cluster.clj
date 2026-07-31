(ns jepsen.openraft.cluster
  (:require [clojure.set :as set]
            [clojure.tools.logging :refer [info]]
            [jepsen.core :as jepsen]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.quorum :as quorum]
            [jepsen.util :as util]))

(defn node-info [test node]
  {:node-id (client/node-host node)
   :api-addr (client/api-endpoint test node)
   :raft-addr (client/raft-addr test node)})

(defn- ready-state? [state]
  (#{"Leader" "Follower"} state))

(defn- vote-leader-id [vote]
  (let [leader-id (:leader_id vote)]
    (or (:node_id leader-id)
        (:voted_for leader-id))))

(defn- committed-leader-vote [metrics leader-id]
  (let [vote (:vote metrics)]
    (when (and (:committed vote)
               (= leader-id (vote-leader-id vote))
               (= leader-id (:current_leader metrics)))
      vote)))

(defn- membership-committed? [metrics]
  (= (get-in metrics [:committed_membership_config :log_id])
     (get-in metrics [:membership_config :log_id])))

(defn- test-node-ids [test]
  (set (map client/node-host (:nodes test))))

(defn- validate-node-ids! [test ids]
  (let [ids (set ids)
        unknown-ids (set/difference ids (test-node-ids test))]
    (when (seq unknown-ids)
      (throw (ex-info "OpenRaft member is not a Jepsen test node"
                      {:member-ids ids
                       :unknown-ids (vec unknown-ids)})))
    ids))

(defn- voter-config-sets [test configs]
  (let [configs (mapv set configs)]
    (validate-node-ids! test (quorum/voter-set configs))
    configs))

(defn node-metrics!
  "Fetches metrics from one node while preserving thread interruption."
  [test node]
  (try
    (client/metrics! (client/api-endpoint test node))
    (catch InterruptedException e
      (.interrupt (Thread/currentThread))
      (throw e))
    (catch Exception e
      (if (= :interrupted (:kind (ex-data e)))
        (do
          (.interrupt (Thread/currentThread))
          (throw (doto (InterruptedException. (ex-message e))
                   (.initCause e))))
        (throw e)))))

(defn- collect-reachable-metrics [test]
  (into {}
        (keep
          (fn [node]
            (try
              [node (node-metrics! test node)]
              (catch InterruptedException e
                (throw e))
              (catch Exception _
                nil)))
          (:nodes test))))

(defn- cluster-status [test]
  (let [metrics (collect-reachable-metrics test)
        leaders (filter (fn [[_ metrics]]
                          (= "Leader" (:state metrics)))
                        metrics)]
    (when (and (= (count (:nodes test)) (count metrics))
               (= 1 (count leaders))
               (every? (comp ready-state? :state val) metrics))
      (let [[leader leader-metrics] (first leaders)
            leader-id (client/node-host leader)
            leader-vote (committed-leader-vote leader-metrics leader-id)]
        (when (and leader-vote
                   (every? #(and (= leader-id (:current_leader %))
                                 (= leader-vote (:vote %)))
                           (vals metrics)))
          {:leader leader
           :metrics metrics})))))

(defn- supported-leader? [test metrics [leader leader-metrics]]
  (let [leader-id (client/node-host leader)
        leader-vote (committed-leader-vote leader-metrics leader-id)
        configs (get-in leader-metrics
                        [:membership_config :membership :configs])
        supporters (->> metrics
                        (keep (fn [[node metrics]]
                                (when (and (= leader-id
                                              (:current_leader metrics))
                                           (= leader-vote (:vote metrics)))
                                  (client/node-host node))))
                        set)]
    (and leader-vote
         (= "Leader" (:state leader-metrics))
         (seq configs)
         (quorum/quorum? configs supporters))))

(defn- serialized-node-id [id]
  (if (keyword? id)
    (name id)
    id))

(defn membership-status
  "Returns the membership view of a leader supported by a voter quorum, or nil."
  [test]
  (let [metrics (collect-reachable-metrics test)
        leaders (filter #(supported-leader? test metrics %) metrics)]
    (when (= 1 (count leaders))
      (let [[leader leader-metrics] (first leaders)
            effective (get leader-metrics :membership_config)
            committed (get leader-metrics :committed_membership_config)
            effective-configs (get-in effective [:membership :configs])
            committed-configs (get-in committed [:membership :configs])
            voter-configs (voter-config-sets test effective-configs)
            committed-voter-configs (voter-config-sets
                                      test
                                      committed-configs)
            voters (quorum/voter-set voter-configs)
            members (->> (get-in effective [:membership :nodes])
                         keys
                         (map serialized-node-id)
                         (validate-node-ids! test))]
        {:leader leader
         :metrics metrics
         :effective-log-id (:log_id effective)
         :committed-log-id (:log_id committed)
         :effective-voter-configs voter-configs
         :committed-voter-configs committed-voter-configs
         :voters voters
         :learners (set/difference members voters)
         :non-members (set/difference (test-node-ids test) members)
         :stable? (and (= 1 (count effective-configs))
                       (= effective-configs committed-configs)
                       (membership-committed? leader-metrics))}))))

(defn await-committed-membership!
  "Waits for a committed membership view from a quorum-supported leader."
  [test]
  (util/await-fn
    #(let [status (membership-status test)]
       (if (and status
                (= (:effective-log-id status)
                   (:committed-log-id status)))
         status
         (throw (ex-info "OpenRaft membership is not committed yet"
                         {:status status}))))
    {:log-message "Waiting for a committed OpenRaft membership"
     :timeout 60000}))

(defn await-stable-membership!
  "Waits for any stable membership, or for the expected voter set."
  ([test]
   (await-stable-membership! test nil))
  ([test expected-voters]
   (let [expected-voters (some-> expected-voters set)]
     (util/await-fn
       #(let [status (membership-status test)]
          (if (and (:stable? status)
                   (or (nil? expected-voters)
                       (= expected-voters (:voters status))))
            status
            (throw (ex-info "OpenRaft membership is not stable yet"
                            {:expected-voters expected-voters
                             :status status}))))
       {:log-message "Waiting for OpenRaft membership to become stable"
        :timeout 60000}))))

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
                         :configs])]
    (when-not (seq configs)
      (throw (ex-info "OpenRaft metrics contain no voter configs"
                      {:leader leader
                       :metrics (get metrics leader)})))
    (voter-config-sets test configs)))

(defn bootstrap! [test]
  (let [leader (jepsen/primary test)
        leader-id (client/node-host leader)
        leader-endpoint (client/api-endpoint test leader)
        learners (remove #{leader} (:nodes test))]
    (info "Initializing OpenRaft cluster on" leader)
    (client/init! leader-endpoint)

    ;; OpenRaft rejects a membership change until the previous membership log
    ;; entry is committed. `init!` only waits for the initial membership entry
    ;; to be flushed, and `current_leader` is set as soon as the vote commits,
    ;; which is still before that entry commits. Waiting for the leader alone
    ;; therefore races with the first add-learner, which fails with
    ;; `ChangeMembershipError::InProgress`.
    (util/await-fn
      #(let [metrics (client/metrics! leader-endpoint)]
         (if (and (= leader-id (:current_leader metrics))
                  (membership-committed? metrics))
           metrics
           (throw (ex-info "Initial OpenRaft leader is not ready yet"
                           {:metrics metrics}))))
      {:log-message "Waiting for initial OpenRaft leader to commit its membership"
       :timeout 60000})

    (doseq [node learners
            :let [{:keys [node-id api-addr raft-addr]} (node-info test node)]]
      (info "Adding OpenRaft learner" node)
      (client/add-learner! leader-endpoint node-id api-addr raft-addr))

    (info "Changing OpenRaft membership to" (:nodes test))
    (client/change-membership! leader-endpoint
                               (mapv client/node-host (:nodes test)))

    (await-ready! test)))
