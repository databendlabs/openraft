(ns jepsen.openraft.cluster
  (:require [clojure.set :as set]
            [clojure.tools.logging :refer [info]]
            [jepsen.core :as jepsen]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.quorum :as quorum])
  (:import (java.util.concurrent ExecutionException
                                 Executors
                                 ExecutorService
                                 Future
                                 TimeUnit)))

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

(defn- committed-vote-key [vote]
  (when (:committed vote)
    [(get-in vote [:leader_id :term])
     (vote-leader-id vote)]))

(defn- latest-committed-vote-key [metrics]
  (->> metrics
       vals
       (keep (comp committed-vote-key :vote))
       sort
       last))

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
    (catch Exception e
      (if (interruption/interruption? e)
        (do
          (.interrupt (Thread/currentThread))
          (throw e))
        (throw e)))))

(def ^:private modeled-metrics-failure-kinds
  #{:http-error
    :invalid-json
    :invalid-response
    :openraft-error
    :request-timeout
    :transport-error
    :unreachable})

(defn- modeled-metrics-failure? [e]
  (contains? modeled-metrics-failure-kinds (:kind (ex-data e))))

(def ^:private probe-drain-seconds 10)

(defn- probe-result
  "Reads one finished probe, surfacing the exception the probe itself threw."
  [^Future probe]
  (try
    (.get probe)
    (catch ExecutionException e
      (throw (.getCause e)))))

(defn- drain-probes!
  "Stops the pool and waits for every probe thread to leave.

  `invokeAll` cancellation and `shutdownNow` only ask a probe to stop, and the
  pool's threads are not daemons, so a scan that returns without waiting leaves
  live threads behind. Each probe is one `client/metrics!` call bounded by its
  five-second request timeout, so a probe that never observes its interrupt
  still ends well inside `probe-drain-seconds`; a pool still running after that
  is reported rather than ignored."
  [^ExecutorService pool]
  (.shutdownNow pool)
  (when-not (.awaitTermination pool probe-drain-seconds TimeUnit/SECONDS)
    (throw (ex-info "OpenRaft metrics probes did not terminate"
                    {:timeout-seconds probe-drain-seconds}))))

(defn- collect-reachable-metrics-from
  "Fetches metrics from nodes at once, dropping the unreachable ones.

  A node whose process is paused answers no request, so its probe ends at
  `client/metrics!`'s five-second request timeout. One Nemesis thread runs
  every fault class, and a sequential scan would hold that thread for one
  timeout per paused node, delaying the cleanup that resumes them.

  `ExecutorService.invokeAll` never drops an interrupt that arrives while this
  thread waits for the probes: it cancels the unfinished ones and rethrows the
  InterruptedException, or it returns while this thread still carries the flag.
  A helper that joins each probe in turn can lose the interrupt instead, when
  the probe it is joining finishes between the throw and the liveness check.

  The scan drains the pool before it restores the flag, so it never returns
  while a probe thread is still running, and the drain is not cut short by the
  interrupt it is about to restore."
  [test nodes]
  (let [^ExecutorService pool (Executors/newCachedThreadPool)
        probes (mapv (fn [node]
                       (fn []
                         (try
                           [node (node-metrics! test node)]
                           (catch InterruptedException e
                             (throw e))
                           (catch Exception e
                             (if (modeled-metrics-failure? e)
                               nil
                               (throw e))))))
                     nodes)
        outcome (try
                  {:metrics (->> (.invokeAll pool probes)
                                 (map probe-result)
                                 (into {} (remove nil?)))}
                  (catch Exception e
                    {:error e}))
        ;; invokeAll clears the flag on this thread when it throws, so this
        ;; drain runs before the flag is put back.
        drain-error (try
                      (drain-probes! pool)
                      nil
                      (catch Exception e
                        e))
        error (or (:error outcome) drain-error)]
    (when error
      (when (interruption/interruption? error)
        (.interrupt (Thread/currentThread)))
      (throw error))
    (:metrics outcome)))

(defn- collect-reachable-metrics [test]
  (collect-reachable-metrics-from test (:nodes test)))

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

(defn- supported-leader? [metrics [leader leader-metrics]]
  (let [leader-id (client/node-host leader)
        leader-vote (committed-leader-vote leader-metrics leader-id)
        latest-vote-key (latest-committed-vote-key metrics)
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
         (= latest-vote-key (committed-vote-key leader-vote))
         (= "Leader" (:state leader-metrics))
         (seq configs)
         (quorum/quorum? configs supporters))))

(defn- supported-leader [metrics]
  (let [leaders (filter #(supported-leader? metrics %) metrics)]
    (when (= 1 (count leaders))
      (first leaders))))

(defn- membership-metrics [test]
  (let [metrics (collect-reachable-metrics test)]
    (if (supported-leader metrics)
      metrics
      ;; Timeouts can make the first scan straddle a leader election.
      (collect-reachable-metrics-from test (keys metrics)))))

(defn- serialized-node-id [id]
  (if (keyword? id)
    (name id)
    id))

(defn membership-status
  "Returns the membership view of a leader supported by a voter quorum, or nil."
  [test]
  (let [metrics (membership-metrics test)]
    (when-let [[leader leader-metrics] (supported-leader metrics)]
      (let [effective (get leader-metrics :membership_config)
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
  (await/until!
   :committed-membership
   #(let [status (membership-status test)]
      (if (and status
               (= (:effective-log-id status)
                  (:committed-log-id status)))
        status
        (await/retry! :committed-membership {:status status})))
   {:log-message "Waiting for a committed OpenRaft membership"
    :timeout 60000}))

(defn await-stable-membership!
  "Waits for any stable membership, or for the expected voter set."
  ([test]
   (await-stable-membership! test nil))
  ([test expected-voters]
   (let [expected-voters (some-> expected-voters set)]
     (await/until!
      :stable-membership
      #(let [status (membership-status test)]
         (if (and (:stable? status)
                  (or (nil? expected-voters)
                      (= expected-voters (:voters status))))
           status
           (await/retry! :stable-membership
                         {:expected-voters expected-voters
                          :status status})))
      {:log-message "Waiting for OpenRaft membership to become stable"
       :timeout 60000}))))

(defn await-ready! [test]
  (await/until!
   :cluster-ready
   #(or (cluster-status test)
        (await/retry! :cluster-ready {}))
   {:log-message "Waiting for every OpenRaft node to agree on a leader"
    :timeout 60000}))

(defn await-node-metrics!
  "Waits for modeled SUT availability while preserving Harness exceptions."
  [test node timeout-ms]
  (await/until!
   :node-metrics
   #(try
      (node-metrics! test node)
      (catch Exception e
        (if (modeled-metrics-failure? e)
          (await/retry! :node-metrics
                        {:node node
                         :failure-kind (:kind (ex-data e))})
          (throw e))))
   {:log-message (str "Waiting for OpenRaft node " node " metrics")
    :timeout timeout-ms}))

(defn await-observed-learner!
  "Waits for a learner in committed membership without retrying Harness errors."
  [test node timeout-ms]
  (await/until!
   :learner-observed
   #(let [status (membership-status test)]
      (if (and (:stable? status)
               (contains? (:learners status) node))
        status
        (await/retry! :learner-observed
                      {:node node
                       :status status})))
   {:log-message (str "Waiting for OpenRaft learner " node
                      " to appear in membership")
    :timeout timeout-ms}))

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
    (await/until!
     :bootstrap-membership
     #(try
        (let [metrics (client/metrics! leader-endpoint)]
          (if (and (= leader-id (:current_leader metrics))
                   (membership-committed? metrics))
            metrics
            (await/retry! :bootstrap-membership {:metrics metrics})))
        (catch Exception e
          (if (modeled-metrics-failure? e)
            (await/retry! :bootstrap-membership
                          {:failure-kind (:kind (ex-data e))})
            (throw e))))
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
