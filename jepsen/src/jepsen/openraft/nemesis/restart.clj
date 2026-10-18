(ns jepsen.openraft.nemesis.restart
  "Kills the leader with SIGKILL and starts it again at once, so that it can
  come back before the followers elect another leader and resume leadership
  in its own term through leader restore.

  The Process Nemesis keeps a killed node down for several seconds, long
  enough for an election, so a restored leader never reuses its log. Here it
  can: an entry it replicated but had not flushed is gone after the kill (the
  test app flushes the RocksDB WAL by hand), and a restored leader that
  assigns the same log id to another entry diverges from the follower that
  kept the first one, which the replica comparison reports."
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.nemesis.outcome :as outcome]))

(def restart-seconds 5)

;; A healthy cluster has a leader again within a few election timeouts; the
;; bound covers a restarted node replaying its log before it answers.
(def ^:private leader-timeout-ms 15000)

(def ^:private kill-results
  #{:killed :target-absent :target-already-exited})

(def ^:private start-results
  #{:already-running :start-confirmed})

(defn- invoke-delegate! [delegate test op]
  (try
    (nemesis/invoke! delegate test op)
    (catch Exception e
      ;; Process control runs on worker threads. Restore the interrupt flag on
      ;; the receiving Nemesis thread too.
      (when (interruption/interruption? e)
        (.interrupt (Thread/currentThread)))
      (throw e))))

(defn- control-result! [db-nemesis test op f node allowed-results]
  (let [results (:value (invoke-delegate! db-nemesis
                                          test
                                          (assoc op :f f :value [node])))]
    (when-not (and (map? results)
                   (= #{node} (set (keys results)))
                   (every? allowed-results (vals results)))
      (throw (ex-info "Unexpected process control result"
                      {:kind :unexpected-process-control-result
                       :operation f
                       :node node
                       :allowed-results allowed-results
                       :result results})))
    (get results node)))

(defn- term [metrics]
  (:current_term metrics))

(defn- await-leader!
  "Waits for a quorum-supported leader; returns its node and term, or nil."
  [test]
  (try
    (await/until! :leader-after-restart
                  #(if-let [{:keys [leader metrics]}
                            (cluster/membership-status test)]
                     {:leader leader
                      :term (term (get metrics leader))}
                     (await/retry! :leader-after-restart {}))
                  {:log-message "Waiting for a leader after a restart"
                   :timeout leader-timeout-ms
                   :retry-interval 50})
    (catch Exception e
      (if (await/condition-timeout? e :leader-after-restart)
        nil
        (throw e)))))

(defn- crash-restart! [db-nemesis active test op]
  (if-let [{:keys [leader metrics]} (cluster/membership-status test)]
    (let [details {:leader leader
                   :term (term (get metrics leader))
                   :last-log-index (get-in metrics [leader :last_log_index])}]
      (info "Killing and restarting the OpenRaft leader" details)
      ;; The node needs a restart from the final cleanup if anything below
      ;; fails between the kill and the start.
      (reset! active leader)
      (let [kill-result (control-result! db-nemesis test op :kill leader
                                         kill-results)]
        (if-not (= :killed kill-result)
          ;; Another fault stopped the leader first and owns its restart.
          (do
            (reset! active nil)
            (outcome/skipped :target-not-running
                             (assoc details :kill-result kill-result)))
          (let [start-result (control-result! db-nemesis test op :start leader
                                              start-results)
                _ (reset! active nil)
                after (await-leader! test)]
            (outcome/installed
             (assoc details
                    :kill-result kill-result
                    :start-result start-result
                    :new-leader (:leader after)
                    :new-term (:term after)
                    :leader-restored? (and (= leader (:leader after))
                                           (= (:term details)
                                              (:term after)))))))))
    (outcome/skipped :no-supported-leader {})))

(defn- cleanup! [db-nemesis active test op]
  (if-let [node @active]
    (let [result (control-result! db-nemesis test op :start node start-results)]
      (reset! active nil)
      (outcome/installed {:node node :start-result result}))
    (outcome/skipped :no-restart-pending {})))

(defrecord RestartNemesis [db-nemesis active]
  nemesis/Nemesis
  (setup! [_ test]
    (RestartNemesis. (nemesis/setup! db-nemesis test) (atom nil)))

  (invoke! [_ test op]
    (assoc op
           :value (case (:f op)
                    :crash-restart-leader (crash-restart! db-nemesis active test op)
                    :finish-restart (cleanup! db-nemesis active test op))))

  (teardown! [_ test]
    (nemesis/teardown! db-nemesis test))

  nemesis/Reflection
  (fs [_]
    #{:crash-restart-leader :finish-restart}))

(defn restart-nemesis [db]
  (RestartNemesis. (combined/db-nemesis db) (atom nil)))

(defn- restart-installed? [value]
  (and (= :installed (:status value))
       (some? (:leader value))
       (contains? value :leader-restored?)))

(defn- recovery-installed? [value]
  (and (= :installed (:status value))
       (:leader value)))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [values (->> history
                        (filter #(= :crash-restart-leader (:f %)))
                        (map :value))
            restarts (filter restart-installed? values)
            restored (filter :leader-restored? restarts)
            errors (->> history
                        (filter #(and (#{:crash-restart-leader :finish-restart}
                                       (:f %))
                                      (or (:error %) (:exception %))))
                        vec)
            malformed (outcome/malformed-outcomes restart-installed? values)
            final-recovery (->> history
                                (filter #(= :await-recovery (:f %)))
                                last)
            recovered? (boolean
                        (recovery-installed? (:value final-recovery)))
            cluster-state (cond
                            (seq errors) :unknown
                            recovered? :intact
                            :else :recovery-pending)]
        ;; Whether a leader comes back in its own term depends on how fast
        ;; it restarts, so a run requires restarts, and reports restores.
        {:valid? (cond
                   (pos? malformed) false
                   (empty? restarts) false
                   (= :intact cluster-state) true
                   (= :recovery-pending cluster-state) false
                   :else :unknown)
         :observed-modes (if (seq restarts) [:leader] [])
         :restarts (count restarts)
         :leader-restores (count restored)
         :malformed-outcomes malformed
         :error-count (count errors)
         :cluster-state cluster-state}))))

(defn restart-package [db]
  {:name :restart
   :interval restart-seconds
   :nemesis (restart-nemesis db)
   :generator (repeat {:type :info :f :crash-restart-leader})
   :final-generator {:type :info :f :finish-restart}
   :checker (openraft-checker/reject-checker-exceptions
             (coverage-checker))})
