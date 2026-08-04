(ns jepsen.openraft.nemesis.process
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.cluster :as cluster]
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

(defrecord ProcessNemesis [delegate killed]
  nemesis/Nemesis
  (setup! [_ test]
    (ProcessNemesis. (nemesis/setup! delegate test) (atom nil)))

  (invoke! [_ test op]
    (case (:f op)
      :kill-process
      (do
        (when @killed
          (throw (ex-info "Processes are already stopped"
                          {:killed @killed})))
        (if-let [status (cluster/membership-status test)]
          (let [mode (:value op)
                disruption (process-disruption test
                                               status
                                               mode
                                               (:nodes test))
                targets (:nodes disruption)]
            (when-not disruption
              (throw (ex-info "Membership has no quorum-safe process targets"
                              {:leader (:leader status)
                               :mode mode
                               :voter-configs (cluster/voter-configs
                                               test
                                               status)})))
            (info "Killing OpenRaft processes" disruption)
            (nemesis/invoke! delegate test
                             (assoc op
                                    :f :kill
                                    :value targets))
            (reset! killed disruption)
            (assoc op :value disruption))
          (do
            (info "Skipping process kill without a quorum-supported leader")
            (assoc op :value :no-supported-leader))))

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
      (do
        (when @paused
          (throw (ex-info "Processes are already paused"
                          {:paused @paused})))
        (if-let [status (cluster/membership-status test)]
          (let [mode (:value op)
                reachable-nodes (->> (:nodes test)
                                     (filter (set (keys (:metrics status))))
                                     vec)]
            (if-let [disruption (process-disruption test
                                                    status
                                                    mode
                                                    reachable-nodes)]
              (let [targets (:nodes disruption)]
                (info "Pausing OpenRaft processes" disruption)
                (nemesis/invoke! delegate test
                                 (assoc op
                                        :f :pause
                                        :value targets))
                (reset! paused disruption)
                (assoc op :value disruption))
              (do
                (info "Skipping process pause without a reachable target"
                      {:mode mode
                       :reachable-nodes reachable-nodes})
                (assoc op :value :no-reachable-pause-target))))
          (do
            (info "Skipping process pause without a quorum-supported leader")
            (assoc op :value :no-supported-leader))))

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

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [observed-modes (->> history
                                (filter #(= :kill-process (:f %)))
                                (keep #(get-in % [:value :mode]))
                                set)
            missing-modes (remove observed-modes required-process-modes)
            cluster-state (reduce
                           (fn [state op]
                             (let [operation-error? (boolean
                                                     (or (:error op)
                                                         (:exception op)))]
                               (cond
                                 (and (= :kill-process (:f op))
                                      operation-error?)
                                 :unknown

                                 (and (= :kill-process (:f op))
                                      (get-in op [:value :mode]))
                                 :degraded

                                 (and (= :restart-process (:f op))
                                      operation-error?)
                                 :unknown

                                 (and (= :await-recovery (:f op))
                                      (get-in op [:value :leader]))
                                 :intact

                                 (and (= :await-recovery (:f op))
                                      operation-error?)
                                 (if (= :degraded state)
                                   :degraded
                                   :unknown)

                                 :else state)))
                           :intact
                           history)
            valid? (cond
                     (seq missing-modes) false
                     (= :intact cluster-state) true
                     (= :degraded cluster-state) false
                     :else :unknown)]
        {:valid? valid?
         :observed-modes (vec (sort observed-modes))
         :missing-modes (vec (sort missing-modes))
         :cluster-state cluster-state}))))

(defn process-package [db]
  {:name :process
   :interval downtime-seconds
   :nemesis (process-nemesis db)
   :generator (process-generator)
   :final-generator {:type :info
                     :f :restart-process}
   :checker (coverage-checker)})

(defn- next-pause-state [expected-nodes state op]
  (let [operation-error? (boolean (or (:error op)
                                      (:exception op)))
        resumed-nodes (get-in op [:value :resumed])]
    (case (:f op)
      :pause-process
      (cond
        operation-error? :unknown
        (get-in op [:value :mode]) :paused
        :else state)

      :resume-process
      (cond
        (= :invoke (:type op)) state
        operation-error? :unknown
        (and (coll? resumed-nodes)
             (= expected-nodes (set resumed-nodes))) :recovery-pending
        :else :unknown)

      :await-recovery
      (cond
        (get-in op [:value :leader]) (if (= :recovery-pending state)
                                       :intact
                                       :unknown)
        operation-error? (if (#{:paused :recovery-pending} state)
                           state
                           :unknown)
        :else state)

      state)))

(defn- pause-coverage-checker []
  (reify checker/Checker
    (check [_ test history _opts]
      (let [expected-nodes (set (:nodes test))
            observed-modes (->> history
                                (filter #(= :pause-process (:f %)))
                                (keep #(get-in % [:value :mode]))
                                set)
            missing-modes (remove observed-modes required-pause-modes)
            cluster-state (reduce (partial next-pause-state expected-nodes)
                                  :intact
                                  history)
            valid? (cond
                     (seq missing-modes) false
                     (= :intact cluster-state) true
                     (#{:paused :recovery-pending} cluster-state) false
                     :else :unknown)]
        {:valid? valid?
         :observed-modes (vec (sort observed-modes))
         :missing-modes (vec (sort missing-modes))
         :cluster-state cluster-state}))))

(defn pause-package [db]
  {:name :pause
   :interval downtime-seconds
   :nemesis (pause-nemesis db)
   :generator (pause-generator)
   :final-generator {:type :info
                     :f :resume-process}
   :checker (pause-coverage-checker)})
