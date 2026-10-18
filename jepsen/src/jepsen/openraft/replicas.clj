(ns jepsen.openraft.replicas
  "Compares the replicas once they have applied the same log.

  Every node reports its `last_applied`, a digest of every entry it applied
  (carried through snapshots), a digest of its key-value map and membership,
  and the log ids it still retains. Once every node reports one common
  `last_applied`, the digests must match and every index two nodes retain
  must hold the same log id. Linearizability checks only what clients read;
  this catches a divergence no read happened to observe, such as an entry a
  leader replicated, lost in a crash, and reused."
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.client :as client]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]))

;; Without client writes only an election appends an entry, and a healthy
;; cluster replicates it within a heartbeat. The bound covers a node that is
;; still installing a snapshot or replaying its log after final recovery.
(def converge-timeout-ms 30000)
(def ^:private max-reported-log-id-mismatches 10)

(def converge-op
  {:type :info
   :f :converge-replicas})

(defn- report! [test node]
  (try
    (client/consistency! (client/api-endpoint test node))
    (catch Exception e
      (if (and (not (interruption/interruption? e))
               (cluster/modeled-metrics-failure? e))
        nil
        (throw e)))))

(defn- reports! [test]
  (into (sorted-map)
        (map (fn [node] [node (report! test node)]))
        (:nodes test)))

(defn- converged? [reports]
  (and (seq reports)
       (every? some? (vals reports))
       (apply = (map :last_applied (vals reports)))))

(defn log-id-mismatches
  "Lists the indexes at which two replicas retain different log ids."
  [reports]
  (->> reports
       (mapcat (fn [[node report]]
                 (map (fn [log-id]
                        {:index (:index log-id)
                         :node node
                         :log-id log-id})
                      (:log_ids report))))
       (group-by :index)
       (keep (fn [[index entries]]
               (let [by-node (into (sorted-map)
                                   (map (juxt :node :log-id))
                                   entries)]
                 (when (< 1 (count (set (vals by-node))))
                   {:index index
                    :log-ids by-node}))))
       (sort-by :index)
       vec))

(defn- by-node [reports k]
  (into (sorted-map)
        (map (fn [[node report]] [node (get report k)]))
        reports))

(defn divergences
  "Compares replica reports taken at one common `last_applied`."
  [reports]
  (let [chains (by-node reports :applied_chain)
        states (by-node reports :state_digest)
        log-ids (log-id-mismatches reports)]
    (cond-> []
      (< 1 (count (set (vals chains))))
      (conj {:kind :applied-chain
             :by-node chains})

      (< 1 (count (set (vals states))))
      (conj {:kind :state-digest
             :by-node states})

      (seq log-ids)
      (conj {:kind :log-ids
             :count (count log-ids)
             :first (vec (take max-reported-log-id-mismatches log-ids))}))))

(defn- summary [report]
  {:last-applied (:last_applied report)
   :applied-chain (:applied_chain report)
   :state-digest (:state_digest report)
   :key-count (:key_count report)
   :retained-log-ids (count (:log_ids report))})

(defn await-convergence!
  "Waits until every node reports the same `last_applied`, then compares them.

  The returned map records whether they converged, a summary of each node's
  last report, and, once converged, the divergences found."
  ([test]
   (await-convergence! test converge-timeout-ms))
  ([test timeout-ms]
   (let [last-reports (atom nil)
         reports (try
                   (await/until!
                    :replicas-converged
                    #(let [reports (reports! test)]
                       (reset! last-reports reports)
                       (if (converged? reports)
                         reports
                         (await/retry! :replicas-converged {})))
                    {:log-message "Waiting for every OpenRaft replica to apply the same log"
                     :timeout timeout-ms
                     :retry-interval 250})
                   (catch Exception e
                     (if (await/condition-timeout? e :replicas-converged)
                       nil
                       (throw e))))]
     (cond-> {:converged? (some? reports)
              :replicas (into (sorted-map)
                              (map (fn [[node report]]
                                     [node (some-> report summary)]))
                              (or reports @last-reports))}
       reports
       (assoc :last-applied (:last_applied (val (first reports)))
              :divergences (divergences reports))))))

(defrecord ReplicaNemesis []
  nemesis/Nemesis
  (setup! [this _test]
    this)

  (invoke! [_ test op]
    (let [result (await-convergence! test)]
      (info "Compared OpenRaft replicas"
            (select-keys result [:converged? :last-applied :divergences]))
      (assoc op :value result)))

  (teardown! [this _test]
    this)

  nemesis/Reflection
  (fs [_]
    #{:converge-replicas}))

(defn package
  "A Nemesis package for `converge-op`, which the lifecycle generator issues
  once the final workload is done."
  []
  {:nemesis (ReplicaNemesis.)
   :perf #{}})

(defn checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [result (->> history
                        (filter #(and (= :converge-replicas (:f %))
                                      (map? (:value %))))
                        (map :value)
                        last)]
        (cond
          (nil? result)
          {:valid? :unknown
           :reason :not-compared}

          (not (:converged? result))
          {:valid? false
           :reason :replicas-did-not-converge
           :replicas (:replicas result)}

          (seq (:divergences result))
          {:valid? false
           :reason :replicas-diverged
           :last-applied (:last-applied result)
           :divergences (:divergences result)
           :replicas (:replicas result)}

          :else
          {:valid? true
           :last-applied (:last-applied result)
           :replicas (:replicas result)})))))
