(ns jepsen.openraft.nemesis.process
  (:require [clojure.tools.logging :refer [info warn]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.quorum :as quorum]))

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
             :active-message "Processes are already stopped"
             :start-message "Killing OpenRaft processes"
             :skip-message
             "Skipping process kill without a quorum-supported leader"}
   :pause {:delegate-f :pause
           :active-key :paused
           :active-message "Processes are already paused"
           :start-message "Pausing OpenRaft processes"
           :skip-message
           "Skipping process pause without a quorum-supported leader"}})

(defn- disruption-spec [kind]
  (or (get disruption-start-specs kind)
      (throw (ex-info "Unknown process disruption kind"
                      {:kind kind}))))

(defn- start-disruption! [delegate active kind test op]
  (let [{:keys [delegate-f active-key active-message
                start-message skip-message]}
        (disruption-spec kind)]
    (when @active
      (throw (ex-info active-message
                      {active-key @active})))
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
            ;; A multi-node disruption may partially succeed before the delegate
            ;; reports an error, so retain every node that may need recovering.
            (reset! active disruption)
            (nemesis/invoke! delegate test
                             (assoc op
                                    :f delegate-f
                                    :value targets))
            (assoc op :value disruption))
          (case kind
            :process
            (throw (ex-info "Membership has no quorum-safe process targets"
                            {:leader (:leader status)
                             :mode mode
                             :voter-configs (cluster/voter-configs test
                                                                   status)}))

            :pause
            (do
              (info "Skipping process pause without a reachable target"
                    {:mode mode
                     :reachable-nodes eligible-nodes})
              (assoc op :value :no-reachable-pause-target)))))
      (do
        (info skip-message)
        (assoc op :value :no-supported-leader)))))

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
        (do
          (info "Restarting OpenRaft processes" {:nodes nodes})
          (nemesis/invoke! delegate test
                           (assoc op
                                  :f :start
                                  :value nodes))
          (reset! killed nil)
          (assoc op :value disruption))
        (assoc op :value :no-processes-killed))))

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
      (let [disruption @paused]
        (info "Resuming all OpenRaft processes" {:nodes (:nodes test)})
        (nemesis/invoke! delegate test
                         (assoc op
                                :f :resume
                                :value (:nodes test)))
        (reset! paused nil)
        (assoc op :value {:paused disruption
                          :resumed (:nodes test)}))))

  (teardown! [_ test]
    (try
      (nemesis/invoke! delegate test
                       {:type :info
                        :f :resume
                        :value (:nodes test)})
      (catch Exception e
        (if (interruption/interruption? e)
          (do
            (.interrupt (Thread/currentThread))
            (throw e))
          (warn e
                "Failed to resume OpenRaft processes during teardown"
                {:nodes (:nodes test)})))
      (finally
        (nemesis/teardown! delegate test))))

  nemesis/Reflection
  (fs [_]
    #{:pause-process :resume-process}))

(defn pause-nemesis [db]
  (PauseNemesis. (combined/db-nemesis db) (atom nil)))

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
               (:exception op))))

(defn- coverage-result
  [required-modes operation-f invalid-states history cluster-state]
  (let [observed-modes (->> history
                            (filter #(= operation-f (:f %)))
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
                                      (get-in op [:value :mode]))
                                 :degraded

                                 (and (= :restart-process (:f op))
                                      error?)
                                 :unknown

                                 (and (= :await-recovery (:f op))
                                      (get-in op [:value :leader]))
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
        resumed-nodes (get-in op [:value :resumed])]
    (case (:f op)
      :pause-process
      (cond
        error? :unknown
        (get-in op [:value :mode]) :paused
        :else state)

      :resume-process
      (cond
        error? :unknown
        (and (coll? resumed-nodes)
             (= expected-nodes (set resumed-nodes))) :recovery-pending
        :else :unknown)

      :await-recovery
      (cond
        (get-in op [:value :leader]) (if (= :recovery-pending state)
                                       :intact
                                       :unknown)
        error? (if (#{:paused :recovery-pending} state)
                 state
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
