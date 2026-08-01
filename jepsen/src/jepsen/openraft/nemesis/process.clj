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
(def healthy-seconds 5)
(def required-process-modes
  #{:leader-killed :leader-survives})
(def required-pause-modes
  #{:leader-paused :leader-unpaused})

(defn- process-targets [nodes configs leader mode]
  (let [voters (quorum/voter-set configs)]
    (when-not (contains? voters leader)
      (throw (ex-info "The current leader is not a voter"
                      {:leader leader
                       :configs configs})))
    (let [fault-sets (quorum/fault-sets configs)
          candidates (cond
                       (#{:leader-killed :leader-paused} mode)
                       (filter #(contains? % leader) fault-sets)

                       (#{:leader-survives :leader-unpaused} mode)
                       (remove #(contains? % leader) fault-sets)

                       :else
                       (throw (ex-info "Unknown process nemesis mode"
                                       {:mode mode})))
          target-set (first (random/shuffle candidates))]
      (when-not target-set
        (throw (ex-info "Membership has no quorum-safe process targets"
                        {:leader leader
                         :mode mode
                         :configs configs})))
      (->> nodes
           (filter target-set)
           vec))))

(defn- process-disruption [test status mode]
  (let [leader (:leader status)
        configs (cluster/voter-configs test status)
        targets (process-targets (:nodes test) configs leader mode)
        voters (quorum/voter-set configs)
        target-set (set targets)
        survivors (->> (:nodes test)
                       (filter voters)
                       (remove target-set)
                       vec)]
    {:mode mode
     :leader leader
     :nodes targets
     :voter-configs configs
     :survivors survivors}))

(defn- await-recovery [test op]
  (let [status (cluster/await-ready! test)]
    (info "OpenRaft cluster recovered with leader" (:leader status))
    (assoc op :value {:leader (:leader status)})))

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
        (let [status (cluster/await-ready! test)
              mode (:value op)
              disruption (process-disruption test status mode)
              targets (:nodes disruption)]
          (info "Killing OpenRaft processes" disruption)
          (nemesis/invoke! delegate test
                           (assoc op
                                  :f :kill
                                  :value targets))
          (reset! killed disruption)
          (assoc op :value disruption)))

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
        (assoc op :value :no-processes-killed))

      :await-recovery
      (await-recovery test op)))

  (teardown! [_ test]
    (nemesis/teardown! delegate test)))

(defn process-nemesis [db]
  (nemesis/validate
   (ProcessNemesis. (combined/db-nemesis db) (atom nil))))

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
        (let [status (cluster/await-ready! test)
              mode (:value op)
              disruption (process-disruption test status mode)
              targets (:nodes disruption)]
          (info "Pausing OpenRaft processes" disruption)
          (nemesis/invoke! delegate test
                           (assoc op
                                  :f :pause
                                  :value targets))
          (reset! paused disruption)
          (assoc op :value disruption)))

      :resume-process
      (let [disruption @paused]
        (info "Resuming all OpenRaft processes" {:nodes (:nodes test)})
        (nemesis/invoke! delegate test
                         (assoc op
                                :f :resume
                                :value (:nodes test)))
        (reset! paused nil)
        (assoc op :value (or disruption :all-processes-resumed)))

      :await-recovery
      (await-recovery test op)))

  (teardown! [_ test]
    (try
      (nemesis/invoke! delegate test
                       {:type :info
                        :f :resume
                        :value (:nodes test)})
      (finally
        (nemesis/teardown! delegate test)))))

(defn pause-nemesis [db]
  (nemesis/validate
   (PauseNemesis. (combined/db-nemesis db) (atom nil))))

(defn- process-generator []
  (gen/cycle
   (gen/phases
    (gen/sleep healthy-seconds)
    {:type :info
     :f :kill-process
     :value :leader-survives}
    (gen/sleep downtime-seconds)
    {:type :info
     :f :restart-process}
    {:type :info
     :f :await-recovery}
    (gen/sleep healthy-seconds)
    {:type :info
     :f :kill-process
     :value :leader-killed}
    (gen/sleep downtime-seconds)
    {:type :info
     :f :restart-process}
    {:type :info
     :f :await-recovery})))

(defn- pause-generator []
  (gen/cycle
   (gen/phases
    (gen/sleep healthy-seconds)
    {:type :info
     :f :pause-process
     :value :leader-unpaused}
    (gen/sleep downtime-seconds)
    {:type :info
     :f :resume-process}
    {:type :info
     :f :await-recovery}
    (gen/sleep healthy-seconds)
    {:type :info
     :f :pause-process
     :value :leader-paused}
    (gen/sleep downtime-seconds)
    {:type :info
     :f :resume-process}
    {:type :info
     :f :await-recovery})))

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
  {:nemesis (process-nemesis db)
   :generator (process-generator)
   :final-generator (gen/phases
                     (gen/once {:type :info
                                :f :restart-process})
                     (gen/once {:type :info
                                :f :await-recovery}))
   :checker (coverage-checker)})

(defn- next-pause-state [state op]
  (let [operation-error? (boolean (or (:error op)
                                      (:exception op)))]
    (case (:f op)
      :pause-process
      (cond
        operation-error? :unknown
        (get-in op [:value :mode]) :paused
        :else state)

      :resume-process
      (cond
        operation-error? :unknown
        (or (= :all-processes-resumed (:value op))
            (get-in op [:value :mode])) :recovery-pending
        :else state)

      :await-recovery
      (cond
        (get-in op [:value :leader]) :intact
        operation-error? (if (#{:paused :recovery-pending} state)
                           state
                           :unknown)
        :else state)

      state)))

(defn- pause-coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [observed-modes (->> history
                                (filter #(= :pause-process (:f %)))
                                (keep #(get-in % [:value :mode]))
                                set)
            missing-modes (remove observed-modes required-pause-modes)
            cluster-state (reduce next-pause-state :intact history)
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
  {:nemesis (pause-nemesis db)
   :generator (pause-generator)
   :final-generator (gen/phases
                     (gen/once {:type :info
                                :f :resume-process})
                     (gen/once {:type :info
                                :f :await-recovery}))
   :checker (pause-coverage-checker)})
