(ns jepsen.openraft.nemesis.process
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.quorum :as quorum]
            [jepsen.openraft.worker :as worker]))

(def downtime-seconds 10)
(def required-process-modes
  #{:leader-killed :leader-survives})
(def required-pause-modes
  #{:leader-paused :leader-unpaused})

(defn- process-targets [eligible-nodes configs leader mode]
  (let [voters (quorum/voter-set configs)]
    (when-not (contains? voters leader)
      (throw (ex-info "The current leader is not a voter"
                      {:leader leader
                       :configs configs})))
    (let [eligible-node-set (set eligible-nodes)
          fault-sets (quorum/fault-sets configs)
          candidates (->> (cond
                            (#{:leader-killed :leader-paused} mode)
                            (filter #(contains? % leader) fault-sets)

                            (#{:leader-survives :leader-unpaused} mode)
                            (remove #(contains? % leader) fault-sets)

                            :else
                            (throw (ex-info "Unknown process nemesis mode"
                                            {:mode mode})))
                          (filter #(every? eligible-node-set %)))
          target-set (first (random/shuffle candidates))]
      (when target-set
        (->> eligible-nodes
             (filter target-set)
             vec)))))

(defn- process-disruption [test status mode eligible-nodes]
  (let [leader (:leader status)
        configs (cluster/voter-configs test status)
        targets (process-targets eligible-nodes configs leader mode)]
    (when targets
      (let [voters (quorum/voter-set configs)
            target-set (set targets)
            survivors (->> (:nodes test)
                           (filter voters)
                           (remove target-set)
                           vec)]
        {:mode mode
         :leader leader
         :nodes targets
         :voter-configs configs
         :survivors survivors}))))

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
                (info "Skipping process kill without a quorum-safe target"
                      details)
                (assoc op
                       :value (outcome/skipped
                               :no-quorum-safe-process-target
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

(defn- legacy-disruption-installed? [required-modes value]
  (and (map? value)
       (required-modes (:mode value))
       (:leader value)
       (coll? (:nodes value))
       (coll? (:voter-configs value))
       (coll? (:survivors value))))

(defn- disruption-installed? [required-modes value]
  (and (outcome/installed-or-legacy?
        value
        (partial legacy-disruption-installed? required-modes))
       (required-modes (:mode value))))

(defn- pause-installed? [value]
  (if (and (map? value) (contains? value :status))
    (let [targets (:nodes value)
          target-set (set targets)
          results (:pause-results value)
          mode (:mode value)
          leader (:leader value)]
      (and (= :installed (:status value))
           (required-pause-modes mode)
           (seq targets)
           (map? results)
           (= (set targets) (set (keys results)))
           (case mode
             :leader-paused
             (and (contains? target-set leader)
                  (= :paused (get results leader)))

             :leader-unpaused
             (and (not (contains? target-set leader))
                  (some #{:paused} (vals results)))

             false)))
    (legacy-disruption-installed? required-pause-modes value)))

(defn- actual-pause-installed? [value]
  (if (and (map? value) (contains? value :status))
    (let [targets (:nodes value)
          results (:pause-results value)]
      (and (= :installed (:status value))
           (seq targets)
           (complete-control-results? (set targets) pause-results results)
           (some #{:paused} (vals results))))
    (legacy-disruption-installed? required-pause-modes value)))

(defn- legacy-resume-installed? [expected-nodes value]
  (and (map? value)
       (contains? value :paused)
       (coll? (:resumed value))
       (= expected-nodes (set (:resumed value)))))

(defn- resume-installed? [expected-nodes value]
  (if (and (map? value) (contains? value :status))
    (let [results (:resume-results value)]
      (and (= :installed (:status value))
           (contains? value :paused)
           (coll? (:resumed value))
           (= expected-nodes (set (:resumed value)))
           (complete-control-results? expected-nodes resume-results results)
           (some #{:resumed} (vals results))))
    (legacy-resume-installed? expected-nodes value)))

(defn- recovery-installed? [value]
  (and (outcome/installed-or-legacy?
        value
        #(and (map? %) (:leader %)))
       (:leader value)))

(defn- process-generator []
  (gen/cycle
   (gen/phases
    {:type :info
     :f :kill-process
     :value :leader-survives}
    {:type :info
     :f :restart-process}
    {:type :info
     :f :kill-process
     :value :leader-killed}
    {:type :info
     :f :restart-process})))

(defn- pause-generator []
  (gen/cycle
   (gen/phases
    {:type :info
     :f :pause-process
     :value :leader-unpaused}
    {:type :info
     :f :resume-process}
    {:type :info
     :f :pause-process
     :value :leader-paused}
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
                                 :intact

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

(defn process-package [db]
  {:name :process
   :interval downtime-seconds
   :nemesis (process-nemesis db)
   :generator (process-generator)
   :final-generator {:type :info
                     :f :restart-process}
   :checker (coverage-checker)})

(defn- next-pause-state [expected-nodes state op]
  (let [error? (operation-error? op)
        status (get-in op [:value :status])]
    (case (:f op)
      :pause-process
      (cond
        error? :unknown
        (actual-pause-installed? (:value op)) :paused
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

(defn pause-package [db]
  {:name :pause
   :interval downtime-seconds
   :nemesis (pause-nemesis db)
   :generator (pause-generator)
   :final-generator {:type :info
                     :f :resume-process}
   :checker (pause-coverage-checker)})
