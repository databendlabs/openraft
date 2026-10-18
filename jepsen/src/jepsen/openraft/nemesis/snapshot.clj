(ns jepsen.openraft.nemesis.snapshot
  "Leaves one follower behind while the leader snapshots and purges its log, so
  that the follower can only catch up through a snapshot install."
  (:require [clojure.set :as set]
            [clojure.string :as str]
            [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [control :as c]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.db :as openraft-db]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.quorum :as quorum]))

(def lag-seconds 6)
(def required-snapshot-modes
  #{:isolate :kill})

;; Emitted by openraft-test-app's install_snapshot.
(def ^:private snapshot-installed-marker "OPENRAFT_JEPSEN_SNAPSHOT_INSTALLED")
(def snapshot-installed-pattern
  (re-pattern snapshot-installed-marker))

;; At the default snapshot threshold the leader snapshots every few seconds of
;; workload, and a five-key snapshot installs at once, so these bound a stuck
;; episode rather than an expected wait.
(def ^:private purge-timeout-ms 20000)
(def ^:private catch-up-timeout-ms 30000)
(def ^:private poll-interval-ms 200)

;; A leader snapshots only as entries commit. When its commit index has not
;; moved for this long, as when other faults hold the quorum down in a chaos
;; run, no purge is coming, and waiting on would keep the shared Nemesis
;; thread from the operations that end those faults.
(def commit-stall-ms 5000)

(def ^:private stop-results
  #{:killed :target-absent :target-already-exited})

(def ^:private start-results
  #{:already-running :start-confirmed})

(def ^:private resume-results
  #{:resumed :target-absent :target-already-exited})

(defn- log-index [log-id]
  (:index log-id))

(defn- lag-candidates
  "Non-leader voters whose loss keeps a quorum of the voters that answer now.

  Counting only reachable voters keeps an episode from starting while other
  faults already hold a quorum down: nothing would commit, the leader could
  not purge, and the stop operation would hold the shared Nemesis thread for
  its whole wait while the restarts that bring the quorum back queue behind
  it."
  [test {:keys [leader metrics] :as status}]
  (let [configs (cluster/voter-configs test status)
        reachable-voters (set/intersection (quorum/voter-set configs)
                                           (set (keys metrics)))]
    (->> (:nodes test)
         (filter reachable-voters)
         (remove #{leader})
         (filter #(quorum/quorum? configs (disj reachable-voters %)))
         vec)))

(defn- await-value
  "Polls f until it returns a truthy value, or returns nil after timeout-ms."
  [condition timeout-ms f]
  (try
    (await/until! condition
                  #(or (f) (await/retry! condition {}))
                  {:log-message (str "Waiting for " (name condition))
                   :timeout timeout-ms
                   :retry-interval poll-interval-ms})
    (catch Exception e
      (if (await/condition-timeout? e condition)
        nil
        (throw e)))))

(defn- log-bound
  "Bounds the index up to which the victim's log can match the leader's, once
  the fault is in place.

  The victim's own last log index is sampled before the fault and is stale by
  the time the fault lands. Entries the leader appends after the fault never
  reach the victim, so the leader's last log index right after the fault
  bounds the match. Without a supported leader the sample is all there is."
  [test sampled]
  (let [{:keys [leader metrics]} (cluster/membership-status test)]
    (max (or sampled -1)
         (or (get-in metrics [leader :last_log_index]) -1))))

(defn- await-purge-past!
  "Waits until the leader has purged the entry after bound, so the victim can
  no longer catch up from the log.

  Returns `{:purged-index i}`, or `{:gave-up reason}` when the leader's commit
  index stalled for `commit-stall-ms`, without a supported leader included,
  or when `purge-timeout-ms` passed."
  [test bound]
  (let [progress (atom {:committed nil :since (System/nanoTime)})
        result (await-value
                :leader-purged-past-victim
                purge-timeout-ms
                #(let [{:keys [leader metrics]} (cluster/membership-status test)
                       purged (log-index (get-in metrics [leader :purged]))
                       committed (log-index (get-in metrics
                                                    [leader :local_committed]))
                       now (System/nanoTime)]
                   (when (and committed
                              (not= committed (:committed @progress)))
                     (reset! progress {:committed committed :since now}))
                   (cond
                     (and purged (> purged bound))
                     {:purged-index purged}

                     (< (* commit-stall-ms 1000000) (- now (:since @progress)))
                     {:gave-up :no-commit-progress}

                     :else
                     nil)))]
    (or result {:gave-up :timeout})))

(defn- await-caught-up!
  "Waits until the victim has applied up to purged-index; returns whether it did."
  [test victim purged-index]
  (boolean
   (await-value :snapshot-victim-caught-up
                catch-up-timeout-ms
                #(try
                   (let [metrics (cluster/node-metrics! test victim)
                         applied (log-index (:last_applied metrics))]
                     (and applied (>= applied purged-index)))
                   (catch Exception e
                     (if (and (not (interruption/interruption? e))
                              (cluster/modeled-metrics-failure? e))
                       false
                       (throw e)))))))

(defn install-count!
  "Counts the snapshot installs node's log records, over SSH. A log the node
  has not created yet counts as none."
  [test node]
  (try
    (let [output (get (c/on-nodes test
                                  [node]
                                  (fn [_test _node]
                                    (c/su
                                     (c/exec :bash
                                             :-c
                                             (str "grep -c "
                                                  snapshot-installed-marker
                                                  " "
                                                  openraft-db/log-file
                                                  " || true")))))
                      node)]
      (or (parse-long (str/trim (str output))) 0))
    (catch Exception e
      (when (interruption/interruption? e)
        (.interrupt (Thread/currentThread)))
      (throw e))))

(defn- invoke-delegate! [delegate test op]
  (try
    (nemesis/invoke! delegate test op)
    (catch Exception e
      ;; Partition and process control run on worker threads. Restore the
      ;; interrupt flag on the receiving Nemesis thread too.
      (when (interruption/interruption? e)
        (.interrupt (Thread/currentThread)))
      (throw e))))

(defn- control-result! [completion victim allowed-results operation]
  (let [results (:value completion)]
    (when-not (and (map? results)
                   (= #{victim} (set (keys results)))
                   (every? allowed-results (vals results)))
      (throw (ex-info "Unexpected process control result"
                      {:kind :unexpected-process-control-result
                       :operation operation
                       :victim victim
                       :allowed-results allowed-results
                       :result results})))
    (get results victim)))

(defn- start-lag! [partitioner db-nemesis active test op]
  (if-let [episode @active]
    (assoc op :value (outcome/skipped :snapshot-lag-active
                                      {:episode episode}))
    (if-let [{:keys [leader metrics] :as status}
             (cluster/membership-status test)]
      (let [mode (:value op)
            victim (first (random/shuffle (lag-candidates test status)))]
        (if-not victim
          (assoc op :value (outcome/skipped
                            :no-snapshot-lag-target
                            {:mode mode
                             :leader leader
                             :voter-configs (cluster/voter-configs test
                                                                   status)}))
          (let [episode {:mode mode
                         :leader leader
                         :victim victim
                         :victim-last-log-index
                         (get-in metrics [victim :last_log_index])
                         :installs-before (install-count! test victim)}]
            (info "Leaving an OpenRaft follower behind the leader's snapshot"
                  episode)
            ;; A failed control command may still have applied the fault, so
            ;; retain the episode for the stop and the final cleanup.
            (reset! active episode)
            (case mode
              :isolate
              (do
                (invoke-delegate! partitioner
                                  test
                                  (assoc op
                                         :f :start
                                         :value (nemesis/complete-grudge
                                                 [[victim]
                                                  (remove #{victim}
                                                          (:nodes test))])))
                (let [episode (assoc episode
                                     :log-bound
                                     (log-bound test
                                                (:victim-last-log-index episode)))]
                  (reset! active episode)
                  (assoc op :value (outcome/installed episode))))

              :kill
              (let [result (control-result!
                            (invoke-delegate! db-nemesis
                                              test
                                              (assoc op
                                                     :f :kill
                                                     :value [victim]))
                            victim
                            stop-results
                            :kill)
                    episode (assoc episode :kill-result result)]
                (if (= :killed result)
                  (let [episode (assoc episode
                                       :log-bound
                                       (log-bound test
                                                  (:victim-last-log-index
                                                   episode)))]
                    (reset! active episode)
                    (assoc op :value (outcome/installed episode)))
                  ;; Another fault stopped the victim first and owns its
                  ;; restart.
                  (do
                    (reset! active nil)
                    (assoc op :value (outcome/skipped :target-not-running
                                                      episode)))))))))
      (assoc op :value (outcome/skipped :no-supported-leader {})))))

(defn- stop-lag! [partitioner db-nemesis active test op]
  (if-let [{:keys [mode victim victim-last-log-index] :as episode} @active]
    (let [wait (when-not (= :cleanup (:value op))
                 (await-purge-past! test
                                    (or (:log-bound episode)
                                        victim-last-log-index
                                        -1)))
          purged (:purged-index wait)
          episode (cond-> (assoc episode
                                 :purged-past-victim? (some? purged)
                                 :purged-index purged)
                    (:gave-up wait) (assoc :purge-wait-gave-up (:gave-up wait)))
          episode (case mode
                    :isolate
                    (do
                      (invoke-delegate! partitioner
                                        test
                                        (assoc op :f :stop :value nil))
                      episode)

                    :kill
                    (do
                      ;; Other faults may have restarted and then paused the
                      ;; victim meanwhile, and a start refuses a paused process.
                      (control-result! (invoke-delegate! db-nemesis
                                                         test
                                                         (assoc op
                                                                :f :resume
                                                                :value [victim]))
                                       victim
                                       resume-results
                                       :resume)
                      (assoc episode
                             :start-result
                             (control-result!
                              (invoke-delegate! db-nemesis
                                                test
                                                (assoc op
                                                       :f :start
                                                       :value [victim]))
                              victim
                              start-results
                              :start))))]
      (reset! active nil)
      (info "Rejoined the OpenRaft follower left behind" episode)
      (assoc op
             :value (outcome/installed
                     (cond-> episode
                       purged
                       (assoc :caught-up? (await-caught-up! test victim purged)
                              :installs-after (install-count! test victim))))))
    (assoc op :value (outcome/skipped :no-snapshot-lag {}))))

(defrecord SnapshotLagNemesis [partitioner db-nemesis active]
  nemesis/Nemesis
  (setup! [_ test]
    (SnapshotLagNemesis. (nemesis/setup! partitioner test)
                         (nemesis/setup! db-nemesis test)
                         (atom nil)))

  (invoke! [_ test op]
    (case (:f op)
      :start-snapshot-lag
      (start-lag! partitioner db-nemesis active test op)

      :stop-snapshot-lag
      (stop-lag! partitioner db-nemesis active test op)))

  (teardown! [_ test]
    (try
      (nemesis/teardown! partitioner test)
      (finally
        (nemesis/teardown! db-nemesis test))))

  nemesis/Reflection
  (fs [_]
    #{:start-snapshot-lag :stop-snapshot-lag}))

(defn snapshot-nemesis [db]
  (SnapshotLagNemesis. (nemesis/partitioner)
                       (combined/db-nemesis db)
                       (atom nil)))

(defn- lag-installed? [value]
  (and (= :installed (:status value))
       (required-snapshot-modes (:mode value))
       (some? (:victim value))))

(defn- recovery-installed? [value]
  (and (= :installed (:status value))
       (:leader value)))

(defn- lag-generator []
  (gen/cycle
   (gen/phases
    {:type :info
     :f :start-snapshot-lag
     :value :isolate}
    {:type :info
     :f :stop-snapshot-lag}
    {:type :info
     :f :start-snapshot-lag
     :value :kill}
    {:type :info
     :f :stop-snapshot-lag})))

(defn- next-cluster-state
  "Applies one nemesis operation to the episode lifecycle state, as the
  partition nemesis does: :intact once the follower is back and the cluster is
  ready, :recovery-pending while an episode or its recovery is open, :unknown
  after an operation error."
  [state op]
  (let [status (get-in op [:value :status])
        operation-error? (boolean (or (:error op)
                                      (:exception op)
                                      (= :indeterminate status)))]
    (case (:f op)
      (:start-snapshot-lag :stop-snapshot-lag)
      (cond
        operation-error? :unknown
        (lag-installed? (:value op)) :recovery-pending
        :else state)

      :await-recovery
      (cond
        operation-error? (if (= :recovery-pending state)
                           :recovery-pending
                           :unknown)
        (= :unknown state) :unknown
        (recovery-installed? (:value op)) :intact
        :else state)

      state)))

(defn- installed-during? [episode]
  (> (or (:installs-after episode) 0)
     (or (:installs-before episode) 0)))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [values-of (fn [f]
                        (->> history
                             (filter #(= f (:f %)))
                             (map :value)))
            start-values (values-of :start-snapshot-lag)
            stop-values (values-of :stop-snapshot-lag)
            purged-stops (->> stop-values
                              (filter lag-installed?)
                              (filter :purged-past-victim?))
            ;; Each episode compares the victim's install count before and
            ;; after, so one install never covers two episodes.
            observed-modes (->> purged-stops
                                (filter installed-during?)
                                (map :mode)
                                set)
            stalled (filterv #(false? (:caught-up? %)) purged-stops)
            missing-modes (remove observed-modes required-snapshot-modes)
            malformed (+ (outcome/malformed-outcomes lag-installed?
                                                     start-values)
                         (outcome/malformed-outcomes lag-installed?
                                                     stop-values))
            cluster-state (reduce next-cluster-state :intact history)
            valid? (cond
                     (pos? malformed) false
                     (seq missing-modes) false
                     (seq stalled) false
                     (= :intact cluster-state) true
                     (= :recovery-pending cluster-state) false
                     :else :unknown)]
        {:valid? valid?
         :observed-modes (vec (sort observed-modes))
         :missing-modes (vec (sort missing-modes))
         :episodes-with-install (count (filter installed-during? purged-stops))
         :stalled-episodes stalled
         :malformed-outcomes malformed
         :cluster-state cluster-state}))))

(defn snapshot-package [db]
  {:name :snapshot
   :interval lag-seconds
   :nemesis (snapshot-nemesis db)
   :generator (lag-generator)
   ;; Final cleanup rejoins the follower at once rather than waiting for a
   ;; purge.
   :final-generator {:type :info
                     :f :stop-snapshot-lag
                     :value :cleanup}
   :checker (openraft-checker/reject-checker-exceptions
             (coverage-checker))})
