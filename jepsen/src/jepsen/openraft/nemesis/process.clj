(ns jepsen.openraft.nemesis.process
  (:require [clojure.set :as set]
            [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.quorum :as quorum]
            [jepsen.openraft.worker :as worker]))

(def downtime-seconds 10)
(def required-process-modes
  #{:random})
(def required-pause-modes
  #{:random})

;; openraft-test-app uses OpenRaft's 300 ms maximum election timeout. Three
;; windows allow a split vote to settle without turning a retained-quorum
;; outage into a silent success.
(def ^:private max-election-timeout-ms 300)
(def ^:private process-availability-window-nanos
  (* 3 max-election-timeout-ms 1000000))

(defn- process-disruption [test status mode eligible-nodes]
  (let [leader (:leader status)
        configs (cluster/voter-configs test status)
        voters (quorum/voter-set configs)
        reachable-node-set (set (keys (:metrics status)))
        reachable-voters (set/intersection voters reachable-node-set)
        targets (some-> (random/nonempty-subset eligible-nodes) vec)]
    (when (seq targets)
      (let [target-set (set targets)
            survivors (->> (:nodes test)
                           (filter reachable-voters)
                           (remove target-set)
                           vec)]
        {:mode mode
         :leader leader
         :nodes targets
         :voter-configs configs
         :reachable-voters (->> (:nodes test)
                                (filter reachable-voters)
                                vec)
         :survivors survivors
         :target-count (count targets)
         :leader-included? (contains? target-set leader)}))))

(def ^:private disruption-start-specs
  {:process {:delegate-f :kill
             :active-key :killed
             :active-reason :processes-already-killed
             :active-message "Processes are already stopped"
             :start-message "Killing OpenRaft processes"
             :skip-message
             "Skipping process kill without a quorum-supported leader"}
   :pause {:delegate-f :pause
           :active-key :paused
           :active-reason :processes-already-paused
           :active-message "Processes are already paused"
           :start-message "Pausing OpenRaft processes"
           :skip-message
           "Skipping process pause without a quorum-supported leader"}})

(defn- disruption-spec [kind]
  (or (get disruption-start-specs kind)
      (throw (ex-info "Unknown process disruption kind"
                      {:kind kind}))))

(def ^:private stop-results
  #{:killed :target-absent :target-already-exited})

(def ^:private start-results
  #{:already-running :start-confirmed})

(def ^:private pause-results
  #{:paused :target-absent :target-already-exited})

(def ^:private resume-results
  #{:resumed :target-absent :target-already-exited})

(defn- complete-control-results?
  [expected-targets allowed-results results]
  (and (map? results)
       (= expected-targets (set (keys results)))
       (every? allowed-results (vals results))))

(defn- invoke-delegate! [delegate test op]
  (try
    (nemesis/invoke! delegate test op)
    (catch Exception e
      ;; c/on-nodes runs multi-node control operations on worker threads.
      ;; Restore the interrupt flag on the receiving Nemesis thread too.
      (when (interruption/interruption? e)
        (.interrupt (Thread/currentThread)))
      (throw e))))

(defn- delegate-results!
  [completion targets allowed-results operation]
  (let [results (:value completion)
        expected-targets (set targets)]
    (when-not (and (map? results)
                   (= expected-targets (set (keys results)))
                   (= (count expected-targets) (count results))
                   (every? allowed-results (vals results)))
      (throw (ex-info "Unexpected process control result"
                      {:kind :unexpected-process-control-result
                       :operation operation
                       :expected-targets expected-targets
                       :allowed-results allowed-results
                       :result results})))
    results))

(defn- control-outcome
  [details completion targets allowed-results operation result-key
   confirmed-result]
  (let [results (delegate-results! completion
                                   targets
                                   allowed-results
                                   operation)
        details (assoc details result-key results)]
    [details
     (cond
       (some #{confirmed-result} (vals results))
       (outcome/installed details)

       (every? #{:target-absent} (vals results))
       (outcome/skipped :target-absent details)

       :else
       (outcome/skipped :target-already-exited details))]))

(defn- stop-outcome [disruption completion]
  (control-outcome disruption
                   completion
                   (:nodes disruption)
                   stop-results
                   :kill
                   :stop-results
                   :killed))

(defn- pause-outcome [disruption completion]
  (control-outcome disruption
                   completion
                   (:nodes disruption)
                   pause-results
                   :pause
                   :pause-results
                   :paused))

(defn- resume-outcome [disruption completion targets]
  (control-outcome {:paused disruption
                    :resumed targets}
                   completion
                   targets
                   resume-results
                   :resume
                   :resume-results
                   :resumed))

(defn- start-disruption! [delegate active kind test op]
  (let [{:keys [delegate-f active-key active-reason active-message
                start-message skip-message]}
        (disruption-spec kind)]
    (if-let [active-disruption @active]
      (do
        (info active-message {active-key active-disruption})
        (assoc op
               :value (outcome/skipped
                       active-reason
                       {:kind kind
                        active-key active-disruption})))
      (if-let [status (cluster/membership-status test)]
        (let [mode (:value op)
              eligible-nodes (if (= :pause kind)
                               (->> (:nodes test)
                                    (filter (set (keys (:metrics status))))
                                    vec)
                               (:nodes test))]
          (if-let [disruption (process-disruption test
                                                  status
                                                  mode
                                                  eligible-nodes)]
            (let [targets (:nodes disruption)]
              (info start-message disruption)
              ;; A multi-node disruption may partially succeed before the
              ;; delegate reports an error, so retain every node that may need
              ;; recovering.
              (reset! active disruption)
              (let [completion (invoke-delegate! delegate
                                                 test
                                                 (assoc op
                                                        :f delegate-f
                                                        :value targets))
                    [active-disruption value]
                    (case kind
                      :process (stop-outcome disruption completion)
                      :pause (pause-outcome disruption completion))]
                (reset! active active-disruption)
                (assoc op :value value)))
            (case kind
              :process
              (let [details {:leader (:leader status)
                             :mode mode
                             :voter-configs (cluster/voter-configs test
                                                                   status)}]
                (info "Skipping process kill without an eligible target"
                      details)
                (assoc op
                       :value (outcome/skipped
                               :no-process-target
                               details)))

              :pause
              (let [details {:mode mode
                             :reachable-nodes eligible-nodes}]
                (info "Skipping process pause without a reachable target"
                      details)
                (assoc op
                       :value (outcome/skipped
                               :no-reachable-pause-target
                               details))))))
        (do
          (info skip-message)
          (assoc op
                 :value (outcome/skipped :no-supported-leader {})))))))

(defrecord ProcessNemesis [delegate killed]
  nemesis/Nemesis
  (setup! [_ test]
    (ProcessNemesis. (nemesis/setup! delegate test) (atom nil)))

  (invoke! [_ test op]
    (case (:f op)
      :kill-process
      (start-disruption! delegate killed :process test op)

      :restart-process
      (if-let [{:keys [nodes] :as disruption} @killed]
        (let [_ (info "Restarting OpenRaft processes" {:nodes nodes})
              completion
              (invoke-delegate! delegate
                                test
                                (assoc op
                                       :f :start
                                       :value nodes))
              results (delegate-results! completion
                                         nodes
                                         start-results
                                         :start)
              value (outcome/installed
                     (assoc disruption :start-results results))]
          (reset! killed nil)
          (assoc op :value value))
        (assoc op
               :value (outcome/skipped :no-processes-killed {})))))

  (teardown! [_ test]
    (nemesis/teardown! delegate test))

  nemesis/Reflection
  (fs [_]
    #{:kill-process :restart-process}))

(defn process-nemesis [db]
  (ProcessNemesis. (combined/db-nemesis db) (atom nil)))

(defrecord PauseNemesis [delegate paused]
  nemesis/Nemesis
  (setup! [_ test]
    (PauseNemesis. (nemesis/setup! delegate test) (atom nil)))

  (invoke! [_ test op]
    (case (:f op)
      :pause-process
      (start-disruption! delegate paused :pause test op)

      :resume-process
      (let [disruption @paused
            targets (:nodes test)]
        (info "Resuming all OpenRaft processes" {:nodes (:nodes test)})
        (let [completion (invoke-delegate! delegate test
                                           (assoc op
                                                  :f :resume
                                                  :value targets))
              [_ value] (resume-outcome disruption completion targets)]
          (reset! paused nil)
          (assoc op :value value)))))

  (teardown! [_ test]
    (try
      (nemesis/invoke! delegate test
                       {:type :info
                        :f :resume
                        :value (:nodes test)})
      (catch Throwable throwable
        (worker/handle-teardown-failure!
         {:action :resume-processes}
         throwable)))
    (try
      (nemesis/teardown! delegate test)
      (catch Throwable throwable
        (worker/handle-teardown-failure!
         {:action :delegate-teardown}
         throwable))))

  nemesis/Reflection
  (fs [_]
    #{:pause-process :resume-process}))

(defn pause-nemesis [db]
  (PauseNemesis. (combined/db-nemesis db) (atom nil)))

(defn- disruption-installed? [required-modes value]
  (and (= :installed (:status value))
       (required-modes (:mode value))))

(defn- pause-installed? [value]
  (let [targets (:nodes value)
        results (:pause-results value)]
    (and (= :installed (:status value))
         (required-pause-modes (:mode value))
         (seq targets)
         (complete-control-results? (set targets) pause-results results)
         (some #{:paused} (vals results)))))

(defn- any-node-paused? [value]
  (let [targets (:nodes value)
        results (:pause-results value)]
    (and (= :installed (:status value))
         (seq targets)
         (complete-control-results? (set targets) pause-results results)
         (some #{:paused} (vals results)))))

(defn- resume-installed? [expected-nodes value]
  (let [results (:resume-results value)]
    (and (= :installed (:status value))
         (contains? value :paused)
         (coll? (:resumed value))
         (= expected-nodes (set (:resumed value)))
         (complete-control-results? expected-nodes resume-results results)
         (some #{:resumed} (vals results)))))

(defn- recovery-installed? [value]
  (and (= :installed (:status value))
       (:leader value)))

(defn- process-generator []
  (gen/cycle
   (gen/phases
    {:type :info
     :f :kill-process
     :value :random}
    {:type :info
     :f :restart-process})))

(defn- pause-generator []
  (gen/cycle
   (gen/phases
    {:type :info
     :f :pause-process
     :value :random}
    {:type :info
     :f :resume-process})))

(defn- operation-error? [op]
  (boolean (or (:error op)
               (:exception op)
               (= :indeterminate (get-in op [:value :status])))))

(defn- coverage-result
  [required-modes operation-f installed? invalid-states history cluster-state]
  (let [observed-modes (->> history
                            (filter #(= operation-f (:f %)))
                            (filter #(installed? (:value %)))
                            (keep #(get-in % [:value :mode]))
                            set)
        missing-modes (remove observed-modes required-modes)
        valid? (cond
                 (seq missing-modes) false
                 (= :intact cluster-state) true
                 (contains? invalid-states cluster-state) false
                 :else :unknown)]
    {:valid? valid?
     :observed-modes (vec (sort observed-modes))
     :missing-modes (vec (sort missing-modes))
     :cluster-state cluster-state}))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [cluster-state (reduce
                           (fn [state op]
                             (let [error? (operation-error? op)]
                               (cond
                                 (and (= :kill-process (:f op))
                                      error?)
                                 :unknown

                                 (and (= :kill-process (:f op))
                                      (disruption-installed?
                                       required-process-modes
                                       (:value op)))
                                 :degraded

                                 (and (= :restart-process (:f op))
                                      error?)
                                 :unknown

                                 (and (= :await-recovery (:f op))
                                      (recovery-installed? (:value op)))
                                 (if (= :unknown state) :unknown :intact)

                                 (and (= :await-recovery (:f op))
                                      error?)
                                 (if (= :degraded state)
                                   :degraded
                                   :unknown)

                                 :else state)))
                           :intact
                           history)]
        (coverage-result required-process-modes
                         :kill-process
                         (partial disruption-installed?
                                  required-process-modes)
                         #{:degraded}
                         history
                         cluster-state)))))

(def ^:private client-completion-types
  #{:ok :fail :info})

(defn- main-client-operation? [op]
  (and (not= :nemesis (:process op))
       (= :main (:phase op))
       (#{:read :write :cas} (:f op))))

(defn- completed-client-attempts [history]
  (:attempts
   (reduce
    (fn [{:keys [pending] :as state} [index op]]
      (cond
        (and (main-client-operation? op)
             (= :invoke (:type op)))
        (assoc-in state [:pending (:process op)]
                  {:index index
                   :op op})

        (and (main-client-operation? op)
             (client-completion-types (:type op)))
        (if-let [{invoke-index :index
                  invocation :op} (get pending (:process op))]
          (-> state
              (update :attempts conj
                      {:process (:process op)
                       :f (:f op)
                       :invoke-index invoke-index
                       :invoke-time (:time invocation)
                       :completion-index index
                       :completion-time (:time op)
                       :type (:type op)
                       :error (:error op)})
              (update :pending dissoc (:process op)))
          state)

        :else state))
    {:pending {}
     :attempts []}
    (map-indexed vector history))))

(defn- installed-random-disruption? [operation-f required-modes op]
  (and (= operation-f (:f op))
       (disruption-installed? required-modes (:value op))))

(defn- recovery-invocation? [operation-f op]
  (and (= operation-f (:f op))
       (nil? (:value op))))

(defn- disruption-episodes
  [disruption-f recovery-f required-modes final-marker history]
  (let [{:keys [active episodes]}
        (reduce
         (fn [{:keys [active] :as state} [index op]]
           (cond
             (installed-random-disruption? disruption-f required-modes op)
             (assoc state
                    :active {:start-index index
                             :started-at (:time op)
                             :details (:value op)})

             (and active (recovery-invocation? recovery-f op))
             (-> state
                 (update :episodes conj
                         (assoc active
                                :end-index index
                                :ended-at (:time op)
                                :final-restart?
                                (boolean (get op final-marker))))
                 (assoc :active nil))

             :else state))
         {:active nil
          :episodes []}
         (map-indexed vector history))]
    (cond-> episodes
      active (conj active))))

(defn- attempt-started-during?
  [{:keys [start-index end-index started-at]} deadline attempt]
  (and (< start-index (:invoke-index attempt))
       (or (nil? end-index)
           (< (:invoke-index attempt) end-index))
       (<= started-at (:invoke-time attempt) deadline)))

(defn- successful-attempt?
  [{:keys [end-index]} deadline attempt]
  (and (or (= :ok (:type attempt))
           (and (= :fail (:type attempt))
                (= :version-mismatch (:error attempt))))
       (or (nil? end-index)
           (< (:completion-index attempt) end-index))
       (<= (:completion-time attempt) deadline)))

(defn- unexpected-write-success?
  [{:keys [end-index]} attempt]
  (and (= :ok (:type attempt))
       (#{:write :cas} (:f attempt))
       (or (nil? end-index)
           (< (:completion-index attempt) end-index))))

(defn- effective-targets [details]
  (let [results (or (:stop-results details)
                    (:pause-results details))
        installed-results #{:killed :paused}]
    (->> results
         (keep (fn [[node result]]
                 (when (installed-results result) node)))
         set)))

(defn- episode-availability [window-nanos attempts episode]
  (let [{:keys [started-at ended-at final-restart? details]} episode
        configs (:voter-configs details)
        targets (effective-targets details)
        reachable-voters (set (:reachable-voters details))
        configured-voters (quorum/voter-set configs)
        survivors (set/difference reachable-voters targets)
        configured-survivors (set/difference configured-voters targets)
        quorum-retained? (quorum/quorum? configs survivors)
        configured-quorum-retained? (quorum/quorum? configs
                                                    configured-survivors)
        deadline (+ started-at window-nanos)
        episode-attempts (filterv (partial attempt-started-during?
                                           episode
                                           Long/MAX_VALUE)
                                  attempts)
        attempts (filterv #(<= (:invoke-time %) deadline)
                          episode-attempts)
        successes (filterv (partial successful-attempt?
                                    episode
                                    deadline)
                           attempts)
        unexpected-successes (when-not configured-quorum-retained?
                               (filterv (partial unexpected-write-success?
                                                 episode)
                                        episode-attempts))
        full-window? (and ended-at (<= deadline ended-at))
        truncated? (and final-restart? (not full-window?))
        reason (cond
                 (seq unexpected-successes)
                 :unexpected-success-without-quorum

                 (not quorum-retained?)
                 nil

                 (seq successes)
                 nil

                 truncated?
                 nil

                 (not full-window?)
                 :insufficient-observation-window

                 (empty? attempts)
                 :no-client-attempts

                 :else
                 :no-success-with-quorum)]
    (cond-> {:started-at started-at
             :ended-at ended-at
             :deadline deadline
             :targets (vec (sort targets))
             :planned-targets (vec (:nodes details))
             :voter-configs configs
             :reachable-voters (vec (sort reachable-voters))
             :survivors (vec (sort survivors))
             :configured-survivors (vec (sort configured-survivors))
             :quorum-retained? quorum-retained?
             :configured-quorum-retained? configured-quorum-retained?
             :evaluable? (boolean (or (not quorum-retained?)
                                      (seq successes)
                                      full-window?))
             :truncated? truncated?
             :attempt-count (count attempts)
             :success-count (count successes)
             :unexpected-success-count (count unexpected-successes)}
      reason (assoc :reason reason))))

(defn- availability-checker
  ([window-nanos]
   (availability-checker window-nanos
                         :kill-process
                         :restart-process
                         required-process-modes
                         :process-final-restart?))
  ([window-nanos disruption-f recovery-f required-modes final-marker]
   (reify checker/Checker
     (check [_ _test history _opts]
       (let [attempts (completed-client-attempts history)
             episodes (->> (disruption-episodes disruption-f
                                                recovery-f
                                                required-modes
                                                final-marker
                                                history)
                           (filter #(number? (:started-at %)))
                           (mapv (partial episode-availability
                                          window-nanos
                                          attempts)))
             failures (filterv :reason episodes)
             retained (filterv :quorum-retained? episodes)
             evaluated-retained (filterv :evaluable? retained)
             valid? (not (seq failures))]
         {:valid? valid?
          :window-nanos window-nanos
          :episode-count (count episodes)
          :quorum-episode-count (count retained)
          :no-quorum-episode-count
          (count (remove :configured-quorum-retained? episodes))
          :indeterminate-quorum-episode-count
          (count (filter #(and (:configured-quorum-retained? %)
                               (not (:quorum-retained? %)))
                         episodes))
          :evaluated-quorum-episode-count (count evaluated-retained)
          :truncated-episode-count (count (filter :truncated? episodes))
          :episodes episodes
          :failures failures})))))

(defn- process-checker []
  (let [coverage (coverage-checker)
        availability (availability-checker
                      process-availability-window-nanos
                      :kill-process
                      :restart-process
                      required-process-modes
                      :process-final-restart?)]
    (reify checker/Checker
      (check [_ test history opts]
        (let [coverage-result (checker/check coverage test history opts)
              availability-result (checker/check availability
                                                 test
                                                 history
                                                 opts)]
          (assoc coverage-result
                 :valid? (checker/merge-valid
                          [(:valid? coverage-result)
                           (:valid? availability-result)])
                 :availability availability-result))))))

(defn process-package [db]
  {:name :process
   :interval downtime-seconds
   :nemesis (process-nemesis db)
   :generator (process-generator)
   :final-generator {:type :info
                     :f :restart-process
                     :process-final-restart? true}
   :checker (openraft-checker/reject-checker-exceptions
             (process-checker))})

(defn- next-pause-state [expected-nodes state op]
  (let [error? (operation-error? op)
        status (get-in op [:value :status])]
    (case (:f op)
      :pause-process
      (cond
        error? :unknown
        (any-node-paused? (:value op)) :paused
        :else state)

      :resume-process
      (cond
        error? :unknown
        (resume-installed? expected-nodes (:value op)) :recovery-pending
        (= :skipped status) state
        :else :unknown)

      :await-recovery
      (cond
        error? (if (#{:paused :recovery-pending} state)
                 state
                 :unknown)
        (recovery-installed? (:value op)) (if (#{:intact
                                                 :recovery-pending} state)
                                            :intact
                                            :unknown)
        :else state)

      state)))

(defn- pause-coverage-checker []
  (reify checker/Checker
    (check [_ test history _opts]
      (let [expected-nodes (set (:nodes test))
            cluster-state (reduce (partial next-pause-state expected-nodes)
                                  :intact
                                  history)]
        (coverage-result required-pause-modes
                         :pause-process
                         pause-installed?
                         #{:paused :recovery-pending}
                         history
                         cluster-state)))))

(defn- pause-checker []
  (let [coverage (pause-coverage-checker)
        availability (availability-checker
                      process-availability-window-nanos
                      :pause-process
                      :resume-process
                      required-pause-modes
                      :pause-final-resume?)]
    (reify checker/Checker
      (check [_ test history opts]
        (let [coverage-result (checker/check coverage test history opts)
              availability-result (checker/check availability
                                                 test
                                                 history
                                                 opts)]
          (assoc coverage-result
                 :valid? (checker/merge-valid
                          [(:valid? coverage-result)
                           (:valid? availability-result)])
                 :availability availability-result))))))

(defn pause-package [db]
  {:name :pause
   :interval downtime-seconds
   :nemesis (pause-nemesis db)
   :generator (pause-generator)
   :final-generator {:type :info
                     :f :resume-process
                     :pause-final-resume? true}
   :checker (openraft-checker/reject-checker-exceptions
             (pause-checker))})
