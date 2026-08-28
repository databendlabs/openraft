(ns jepsen.openraft.nemesis.process-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [db :as db]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.generator.test :as gen-test]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.nemesis :as openraft-nemesis]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.nemesis.process :as process]
            [jepsen.openraft.quorum :as quorum]
            [jepsen.openraft.worker :as worker]))

(def voters ["n1" "n2" "n3" "n4" "n5"])

(defn- healthy-status [nodes leader]
  {:leader leader
   :metrics (zipmap nodes (repeat {}))})

(defn- installed [details]
  (assoc details :status :installed))

(defn- skipped [reason details]
  (assoc details :status :skipped :reason reason))

(defn- indeterminate [reason details]
  (assoc details :status :indeterminate :reason reason))

(defn- delegate-completion [op]
  (case (:f op)
    :kill (assoc op :value (zipmap (:value op) (repeat :killed)))
    :start (assoc op :value (zipmap (:value op)
                                    (repeat :start-confirmed)))
    :pause (assoc op :value (zipmap (:value op) (repeat :paused)))
    :resume (assoc op :value (zipmap (:value op) (repeat :resumed)))
    op))

(defn- recording-nemesis [invocations]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! invocations conj op)
      (delegate-completion op))
    (teardown! [this _test]
      this)))

(defn- failing-nemesis [invocations failing-f error]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! invocations conj op)
      (when (= failing-f (:f op))
        (throw error))
      (delegate-completion op))
    (teardown! [this _test]
      this)))

(defn- failing-resume-nemesis
  ([events resume-error]
   (failing-resume-nemesis events resume-error nil))
  ([events resume-error teardown-error]
   (reify nemesis/Nemesis
     (setup! [this _test]
       this)
     (invoke! [_ _test op]
       (swap! events conj [(:f op) (:value op)])
       (throw resume-error))
     (teardown! [this _test]
       (swap! events conj :teardown)
       (when teardown-error
         (throw teardown-error))
       this))))

(defn- failing-teardown-nemesis [events error]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! events conj [(:f op) (:value op)])
      (delegate-completion op))
    (teardown! [_ _test]
      (swap! events conj :teardown)
      (throw error))))

(deftest pause-selects-any-nonempty-node-subset
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        subject (process/->PauseNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3" "n4" "n5"]}
        configs [#{"n1" "n2" "n3"}]
        selected ["n2" "n4" "n5"]]
    (with-redefs [cluster/membership-status
                  (constantly (healthy-status (:nodes test) "n1"))
                  cluster/voter-configs
                  (fn [_test _status] configs)
                  random/nonempty-subset
                  (fn [nodes]
                    (is (= (:nodes test) nodes))
                    selected)]
      (let [completion (nemesis/invoke! subject
                                        test
                                        {:type :info
                                         :f :pause-process
                                         :value :random})
            value (:value completion)]
        (is (= :installed (:status value)))
        (is (= selected (:nodes value)))
        (is (= 3 (:target-count value)))
        (is (= :majority (:target-category value)))
        (is (false? (:leader-included? value)))
        (is (= configs (:voter-configs value)))
        (is (= ["n1" "n2" "n3"] (:reachable-voters value)))
        (is (not (contains? value :survivors)))
        (is (= [[:pause selected]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest process-selects-any-nonempty-node-subset
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        subject (process/->ProcessNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3" "n4" "n5"]}
        configs [#{"n1" "n2" "n3"}]
        selected ["n2" "n4" "n5"]]
    (with-redefs [cluster/membership-status
                  (constantly (healthy-status (:nodes test) "n1"))
                  cluster/voter-configs
                  (fn [_test _status] configs)
                  random/nonempty-subset
                  (fn [nodes]
                    (is (= (:nodes test) nodes))
                    selected)]
      (let [completion (nemesis/invoke! subject
                                        test
                                        {:type :info
                                         :f :kill-process
                                         :value :random})
            value (:value completion)]
        (is (= :installed (:status value)))
        (is (= selected (:nodes value)))
        (is (= 3 (:target-count value)))
        (is (= :majority (:target-category value)))
        (is (false? (:leader-included? value)))
        (is (= configs (:voter-configs value)))
        (is (= ["n1" "n2" "n3"] (:reachable-voters value)))
        (is (not (contains? value :survivors)))
        (is (= [[:kill selected]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest categorizes-target-scale
  (is (= [:one :minority :majority :majority :all]
         (mapv (partial outcome/target-category 5)
               (range 1 6)))))

(deftest process-generator-repeats-random-kill-episodes
  (let [invocations (atom [])]
    (gen-test/simulate
     (gen/limit 4 (#'process/process-generator))
     (fn [_context operation]
       (swap! invocations conj operation)
       (assoc operation :type :info)))
    (is (= [[:kill-process :random]
            [:restart-process nil]
            [:kill-process :random]
            [:restart-process nil]]
           (mapv (juxt :f :value) @invocations)))))

(deftest process-final-restart-uses-a-private-marker
  (let [operation (:final-generator (process/process-package nil))]
    (is (:process-final-restart? operation))
    (is (not (contains? operation :final?)))))

(deftest pause-final-resume-uses-a-private-marker
  (let [operation (:final-generator (process/pause-package nil))]
    (is (:pause-final-resume? operation))
    (is (not (contains? operation :final?)))))

(deftest restarts-the-processes-that-were-killed
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        subject (process/->ProcessNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3"]}
        status (healthy-status (:nodes test) "n1")]
    (with-redefs [cluster/membership-status (fn [_test] status)
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])
                  random/nonempty-subset (constantly ["n1"])]
      (let [killed (nemesis/invoke!
                    subject
                    test
                    {:type :info
                     :f :kill-process
                     :value :random})
            restarted (nemesis/invoke!
                       subject
                       test
                       {:type :info
                        :f :restart-process})]
        (is (= ["n1"] (get-in killed [:value :nodes])))
        (is (= ["n1"] (get-in restarted [:value :nodes])))
        (is (= :installed (get-in killed [:value :status])))
        (is (= :installed (get-in restarted [:value :status])))
        (is (= {"n1" :killed}
               (get-in killed [:value :stop-results])))
        (is (= {"n1" :start-confirmed}
               (get-in restarted [:value :start-results])))
        (is (= [[:kill ["n1"]]
                [:start ["n1"]]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest derives-process-outcomes-from-confirmed-node-results
  (let [test {:nodes ["n1" "n2" "n3"]}
        status (healthy-status (:nodes test) "n1")
        invoke-with-results
        (fn [results]
          (let [delegate (reify nemesis/Nemesis
                           (setup! [this _test] this)
                           (invoke! [_ _test op]
                             (assoc op :value results))
                           (teardown! [this _test] this))
                subject (process/->ProcessNemesis delegate (atom nil))]
            (with-redefs [cluster/membership-status (constantly status)
                          cluster/voter-configs
                          (fn [_test _status] [(set (:nodes test))])
                          random/nonempty-subset (constantly ["n1"])]
              (nemesis/invoke! subject
                               test
                               {:type :info
                                :f :kill-process
                                :value :random}))))]
    (testing "every pre-probe absence skips the kill"
      (let [value (:value (invoke-with-results {"n1" :target-absent}))]
        (is (= :skipped (:status value)))
        (is (= :target-absent (:reason value)))))

    (testing "an explicit exit race skips the kill"
      (let [value (:value
                   (invoke-with-results {"n1" :target-already-exited}))]
        (is (= :skipped (:status value)))
        (is (= :target-already-exited (:reason value)))))

    (testing "any confirmed kill installs a multi-node disruption"
      (let [five-node-test {:nodes voters}
            results {"n2" :killed
                     "n3" :target-absent}
            delegate (reify nemesis/Nemesis
                       (setup! [this _test] this)
                       (invoke! [_ _test op]
                         (assoc op :value results))
                       (teardown! [this _test] this))
            subject (process/->ProcessNemesis delegate (atom nil))]
        (with-redefs [cluster/membership-status
                      (constantly
                       (healthy-status (:nodes five-node-test) "n1"))
                      cluster/voter-configs
                      (fn [_test _status] [(set voters)])
                      random/nonempty-subset (constantly ["n2" "n3"])]
          (let [value (:value
                       (nemesis/invoke! subject
                                        five-node-test
                                        {:type :info
                                         :f :kill-process
                                         :value :random}))]
            (is (= :installed (:status value)))
            (is (= results (:stop-results value)))))))))

(deftest derives-pause-outcomes-from-confirmed-node-results
  (let [test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}
        invoke-with-result
        (fn [result]
          (let [delegate (reify nemesis/Nemesis
                           (setup! [this _test] this)
                           (invoke! [_ _test op]
                             (assoc op
                                    :value (zipmap (:value op)
                                                   (repeat result))))
                           (teardown! [this _test] this))
                subject (process/->PauseNemesis delegate (atom nil))]
            (with-redefs [cluster/membership-status (constantly status)
                          cluster/voter-configs
                          (fn [_test _status] [(set (:nodes test))])
                          random/nonempty-subset (constantly ["n1"])]
              (nemesis/invoke! subject
                               test
                               {:type :info
                                :f :pause-process
                                :value :random}))))]
    (testing "every pre-probe absence skips the pause"
      (let [value (:value (invoke-with-result :target-absent))]
        (is (= :skipped (:status value)))
        (is (= :target-absent (:reason value)))
        (is (= {"n1" :target-absent} (:pause-results value)))))

    (testing "an explicit exit race skips the pause"
      (let [value (:value (invoke-with-result :target-already-exited))]
        (is (= :skipped (:status value)))
        (is (= :target-already-exited (:reason value)))
        (is (= {"n1" :target-already-exited}
               (:pause-results value)))))

    (testing "a confirmed pause installs the disruption"
      (let [value (:value (invoke-with-result :paused))]
        (is (= :installed (:status value)))
        (is (= {"n1" :paused} (:pause-results value)))))))

(deftest derives-resume-outcomes-from-confirmed-node-results
  (let [test {:nodes ["n1" "n2" "n3"]}
        disruption {:mode :random
                    :leader "n1"
                    :nodes ["n1"]
                    :voter-configs [(set (:nodes test))]
                    :pause-results {"n1" :paused}}
        invoke-with-results
        (fn [results]
          (let [delegate (reify nemesis/Nemesis
                           (setup! [this _test] this)
                           (invoke! [_ _test op]
                             (assoc op :value results))
                           (teardown! [this _test] this))
                active (atom disruption)
                subject (process/->PauseNemesis delegate active)
                completion (nemesis/invoke! subject
                                            test
                                            {:type :info
                                             :f :resume-process})]
            [completion @active]))]
    (testing "every absent target skips the resume"
      (let [results (zipmap (:nodes test) (repeat :target-absent))
            [completion active] (invoke-with-results results)
            value (:value completion)]
        (is (= :skipped (:status value)))
        (is (= :target-absent (:reason value)))
        (is (= results (:resume-results value)))
        (is (nil? active))))

    (testing "an explicit exit race skips the resume"
      (let [results {"n1" :target-absent
                     "n2" :target-already-exited
                     "n3" :target-absent}
            [completion active] (invoke-with-results results)
            value (:value completion)]
        (is (= :skipped (:status value)))
        (is (= :target-already-exited (:reason value)))
        (is (= results (:resume-results value)))
        (is (nil? active))))

    (testing "any confirmed resume installs multi-node recovery"
      (let [results {"n1" :resumed
                     "n2" :target-absent
                     "n3" :target-already-exited}
            [completion active] (invoke-with-results results)
            value (:value completion)]
        (is (= :installed (:status value)))
        (is (= results (:resume-results value)))
        (is (= disruption (:paused value)))
        (is (nil? active))))))

(deftest rejects-malformed-process-control-results-and-retains-recovery-state
  (let [test {:nodes ["n1" "n2" "n3"]}
        invocations (atom 0)
        delegate (reify nemesis/Nemesis
                   (setup! [this _test] this)
                   (invoke! [_ _test op]
                     (swap! invocations inc)
                     (assoc op :value {}))
                   (teardown! [this _test] this))
        active (atom nil)
        subject (process/->ProcessNemesis delegate active)]
    (with-redefs [cluster/membership-status
                  (constantly (healthy-status (:nodes test) "n1"))
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])
                  random/nonempty-subset (constantly ["n1"])]
      (let [error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :kill-process
                                      :value :random})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= ["n1"] (:nodes @active))))

      (let [error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :restart-process})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= ["n1"] (:nodes @active)))
        (is (= 2 @invocations))))))

(deftest rejects-malformed-pause-control-results-and-retains-recovery-state
  (let [test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}
        delegate (reify nemesis/Nemesis
                   (setup! [this _test] this)
                   (invoke! [_ _test op]
                     (assoc op :value {}))
                   (teardown! [this _test] this))
        active (atom nil)
        subject (process/->PauseNemesis delegate active)]
    (with-redefs [cluster/membership-status (constantly status)
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])
                  random/nonempty-subset (constantly ["n1"])]
      (let [error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :pause-process
                                      :value :random})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= ["n1"] (:nodes @active))))

      (let [active-before-resume @active
            error (try
                    (nemesis/invoke! subject
                                     test
                                     {:type :info
                                      :f :resume-process})
                    nil
                    (catch Exception e
                      e))]
        (is (= :unexpected-process-control-result
               (:kind (ex-data error))))
        (is (= active-before-resume @active))))))

(deftest multi-node-control-restores-the-receiving-thread-interrupt-flag
  (let [test {:nodes voters
              :sessions (zipmap voters (repeat :unused-session))}
        planned #{"n2" "n3"}
        disruption {:mode :random
                    :leader "n1"
                    :nodes ["n2" "n3"]
                    :voter-configs [(set voters)]
                    :pause-results {"n2" :paused "n3" :paused}}
        prefer-planned (fn [candidates]
                         (cons planned (remove #{planned} candidates)))
        cases
        [[:kill
          #(InterruptedException. "interrupted")
          process/process-nemesis
          nil
          {:type :info :f :kill-process :value :random}]
         [:start
          #(java.io.InterruptedIOException. "interrupted")
          process/process-nemesis
          :killed
          {:type :info :f :restart-process}]
         [:pause
          #(java.nio.channels.ClosedByInterruptException.)
          process/pause-nemesis
          nil
          {:type :info :f :pause-process :value :random}]
         [:resume
          #(ex-info "interrupted" {:kind :interrupted})
          process/pause-nemesis
          :paused
          {:type :info :f :resume-process}]]]
    (doseq [[label make-error make-subject active-key op] cases]
      (testing (name label)
        (Thread/interrupted)
        (try
          (let [error (make-error)
                database (reify
                           db/Process
                           (kill! [_ _test _node]
                             (throw error))
                           (start! [_ _test _node]
                             (throw error))

                           db/Pause
                           (pause! [_ _test _node]
                             (throw error))
                           (resume! [_ _test _node]
                             (throw error)))
                subject (make-subject database)
                _ (when active-key
                    (reset! (get subject active-key) disruption))
                [thrown interrupted?]
                (with-redefs [cluster/membership-status
                              (constantly
                               {:leader "n1"
                                :metrics (zipmap voters (repeat {}))})
                              cluster/voter-configs
                              (fn [_test _status] [(set voters)])
                              random/nonempty-subset
                              (constantly ["n2" "n3"])
                              random/shuffle prefer-planned]
                  (try
                    (nemesis/invoke! subject test op)
                    [nil (.isInterrupted (Thread/currentThread))]
                    (catch Exception e
                      [e (.isInterrupted (Thread/currentThread))])))]
            (is (identical? error thrown))
            (is interrupted?))
          (finally
            (Thread/interrupted)))))))

(deftest restarts-all-planned-processes-after-a-kill-error
  (let [invocations (atom [])
        delegate (failing-nemesis invocations
                                  :kill
                                  (ex-info "kill failed" {}))
        subject (process/->ProcessNemesis delegate (atom nil))
        test {:nodes voters}
        planned ["n2" "n3"]]
    (with-redefs [cluster/membership-status
                  (constantly (healthy-status (:nodes test) "n1"))
                  cluster/voter-configs (fn [_test _status]
                                          [(set voters)])
                  random/nonempty-subset (constantly planned)]
      (is (thrown-with-msg?
           clojure.lang.ExceptionInfo
           #"kill failed"
           (nemesis/invoke! subject
                            test
                            {:type :info
                             :f :kill-process
                             :value :random})))
      (let [restarted (nemesis/invoke! subject
                                       test
                                       {:type :info :f :restart-process})]
        (is (= ["n2" "n3"] (get-in restarted [:value :nodes])))
        (is (= [[:kill ["n2" "n3"]]
                [:start ["n2" "n3"]]]
               (mapv (juxt :f :value) @invocations)))))))

(deftest skips-a-pause-after-a-pause-error
  (let [invocations (atom [])
        delegate (failing-nemesis invocations
                                  :pause
                                  (ex-info "pause failed" {}))
        subject (process/->PauseNemesis delegate (atom nil))
        test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}]
    (with-redefs [cluster/membership-status (fn [_test] status)
                  cluster/voter-configs (fn [_test _status]
                                          [(set (:nodes test))])
                  random/nonempty-subset (constantly ["n1"])]
      (is (thrown-with-msg?
           clojure.lang.ExceptionInfo
           #"pause failed"
           (nemesis/invoke! subject test {:type :info
                                          :f :pause-process
                                          :value :random})))
      (let [skipped-value
            (:value (nemesis/invoke! subject test {:type :info
                                                   :f :pause-process
                                                   :value :random}))]
        (is (= :skipped (:status skipped-value)))
        (is (= :processes-already-paused (:reason skipped-value))))
      (is (= [[:pause ["n1"]]]
             (mapv (juxt :f :value) @invocations)))
      (let [resumed (nemesis/invoke! subject
                                     test
                                     {:type :info :f :resume-process})]
        (is (= {:mode :random
                :leader "n1"
                :nodes ["n1"]
                :voter-configs [#{"n1" "n2" "n3"}]
                :reachable-voters ["n1" "n2" "n3"]
                :target-count 1
                :target-category :one
                :leader-included? true}
               (get-in resumed [:value :paused])))))))

(deftest records-pause-recovery-history
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        subject (process/->PauseNemesis delegate (atom nil))
        recovery (openraft-nemesis/->RecoveryNemesis)
        test {:nodes ["n1" "n2" "n3"]}
        status {:leader "n1"
                :metrics (zipmap (:nodes test) (repeat {}))}
        coverage-checker (#'process/pause-coverage-checker)
        invoke! (fn
                  ([f]
                   (nemesis/invoke! subject test {:type :info :f f}))
                  ([f value]
                   (nemesis/invoke! subject test {:type :info
                                                  :f f
                                                  :value value})))
        recover! #(nemesis/invoke! recovery
                                   test
                                   {:type :info
                                    :f :await-recovery})]
    (with-redefs [cluster/membership-status (fn [_test] status)
                  cluster/await-ready! (fn [_test] status)
                  cluster/voter-configs
                  (fn [_test _status] [(set (:nodes test))])
                  random/nonempty-subset (constantly ["n1"])]
      (let [leader-pause (invoke! :pause-process :random)
            duplicate-pause (invoke! :pause-process :random)
            leader-resume (invoke! :resume-process)
            leader-recovery (recover!)
            follower-pause (invoke! :pause-process :random)
            follower-resume (invoke! :resume-process)
            follower-recovery (recover!)
            cleanup-resume (invoke! :resume-process)
            cleanup-recovery (recover!)
            history [leader-pause
                     leader-resume
                     leader-recovery
                     follower-pause
                     follower-resume
                     follower-recovery
                     cleanup-resume
                     cleanup-recovery]
            result (checker/check coverage-checker test history {})]
        (is (= :skipped (get-in duplicate-pause [:value :status])))
        (is (= :processes-already-paused
               (get-in duplicate-pause [:value :reason])))
        (is (= {:paused (dissoc (:value leader-pause) :status)
                :resumed (:nodes test)
                :resume-results (zipmap (:nodes test) (repeat :resumed))
                :status :installed}
               (:value leader-resume)))
        (is (= {:paused (dissoc (:value follower-pause) :status)
                :resumed (:nodes test)
                :resume-results (zipmap (:nodes test) (repeat :resumed))
                :status :installed}
               (:value follower-resume)))
        (is (= {:paused nil
                :resumed (:nodes test)
                :resume-results (zipmap (:nodes test) (repeat :resumed))
                :status :installed}
               (:value cleanup-resume)))
        (is (= [[:pause (get-in leader-pause [:value :nodes])]
                [:resume (:nodes test)]
                [:pause (get-in follower-pause [:value :nodes])]
                [:resume (:nodes test)]
                [:resume (:nodes test)]]
               (mapv (juxt :f :value) @invocations)))
        (is (= {:valid? true
                :cluster-state :intact}
               (select-keys result [:valid? :cluster-state])))))))

(deftest pauses-only-reachable-voters-after-a-process-kill
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        process-nemesis (process/->ProcessNemesis delegate (atom nil))
        pause-nemesis (process/->PauseNemesis delegate (atom nil))
        test {:nodes voters}
        metrics (atom (zipmap voters (repeat {})))
        prefer-n2 (fn [candidates]
                    (let [candidates (vec candidates)
                          preferred #{"n2"}]
                      (if (some #{preferred} candidates)
                        (cons preferred (remove #{preferred} candidates))
                        candidates)))]
    (with-redefs [cluster/membership-status (fn [_test]
                                              {:leader "n1"
                                               :metrics @metrics})
                  cluster/voter-configs (fn [_test _status]
                                          [(set voters)])
                  random/nonempty-subset
                  (fn [nodes]
                    (if (some #{"n2"} nodes)
                      ["n2"]
                      [(first nodes)]))
                  random/shuffle prefer-n2]
      (let [killed (nemesis/invoke! process-nemesis
                                    test
                                    {:type :info
                                     :f :kill-process
                                     :value :random})
            killed-nodes (set (get-in killed [:value :nodes]))]
        (is (= #{"n2"} killed-nodes))
        (swap! metrics #(apply dissoc % killed-nodes))
        (let [paused (nemesis/invoke! pause-nemesis
                                      test
                                      {:type :info
                                       :f :pause-process
                                       :value :random})
              paused-nodes (get-in paused [:value :nodes])
              delegated-pause (first (filter #(= :pause (:f %))
                                             @invocations))]
          (is (seq paused-nodes))
          (is (every? (set (keys @metrics)) paused-nodes))
          (is (= paused-nodes (:value delegated-pause))))))))

(deftest pause-may-target-the-only-reachable-voter
  (let [invocations (atom [])
        subject (process/->PauseNemesis
                 (recording-nemesis invocations)
                 (atom nil))
        test {:nodes ["n1"]}
        status {:leader "n1"
                :metrics {"n1" {}}}]
    (with-redefs [cluster/membership-status (constantly status)
                  cluster/voter-configs (fn [_test _status]
                                          [#{"n1"}])]
      (let [completion (nemesis/invoke! subject
                                        test
                                        {:type :info
                                         :f :pause-process
                                         :value :random})
            result (checker/check (#'process/pause-coverage-checker)
                                  test
                                  [completion]
                                  {})]
        (is (= :installed (get-in completion [:value :status])))
        (is (= ["n1"] (get-in completion [:value :nodes])))
        (is (= [[:pause ["n1"]]]
               (mapv (juxt :f :value) @invocations)))
        (is (= [:random] (:observed-modes result)))))))

(deftest teardown-cleanup-failure-records-a-harness-failure
  (let [failure-state (harness/failure-state)
        events (atom [])
        error (ex-info "unreachable" {:kind :unreachable})
        delegate (failing-resume-nemesis
                  events
                  error)
        subject (worker/wrap-nemesis-teardown
                 failure-state
                 :pause
                 (process/->PauseNemesis delegate (atom nil)))
        test {:nodes ["n1" "n2" "n3"]}]
    (nemesis/teardown! subject test)
    (is (= [[:resume (:nodes test)] :teardown]
           @events))
    (let [failure (harness/primary-failure failure-state)]
      (is (= :nemesis (:source failure)))
      (is (= {:phase :teardown
              :component :pause
              :action :resume-processes
              :nodes (:nodes test)}
             (:context failure)))
      (is (identical? error (:throwable failure))))))

(deftest teardown-retains-each-stage-failure
  (let [failure-state (harness/failure-state)
        events (atom [])
        resume-error (RuntimeException. "resume failed")
        teardown-error (RuntimeException. "delegate teardown failed")
        delegate (failing-resume-nemesis
                  events
                  resume-error
                  teardown-error)
        subject (worker/wrap-nemesis-teardown
                 failure-state
                 :pause
                 (process/->PauseNemesis delegate (atom nil)))
        test {:nodes ["n1" "n2" "n3"]}]
    (nemesis/teardown! subject test)
    (is (= [[:resume (:nodes test)] :teardown] @events))
    (let [primary (harness/primary-failure failure-state)
          [secondary] (harness/secondary-failures failure-state)]
      (is (= :resume-processes (get-in primary [:context :action])))
      (is (identical? resume-error (:throwable primary)))
      (is (= :delegate-teardown (get-in secondary [:context :action])))
      (is (identical? teardown-error (:throwable secondary))))))

(deftest teardown-preserves-interruptions
  (doseq [[label error]
          [[:interrupted-exception
            (InterruptedException. "interrupted")]
           [:interrupted-io
            (java.io.InterruptedIOException. "interrupted")]
           [:closed-by-interrupt
            (java.nio.channels.ClosedByInterruptException.)]
           [:wrapped
            (ex-info "interrupted" {:kind :interrupted})]]]
    (testing (name label)
      (Thread/interrupted)
      (let [events (atom [])
            delegate (failing-resume-nemesis events error)
            subject (process/->PauseNemesis delegate (atom nil))
            test {:nodes ["n1" "n2" "n3"]}]
        (try
          (let [thrown (try
                         (nemesis/teardown! subject test)
                         nil
                         (catch Exception e
                           e))
                interrupted? (.isInterrupted (Thread/currentThread))]
            (is (identical? error thrown))
            (is interrupted?)
            (is (= [[:resume (:nodes test)]]
                   @events)))
          (finally
            (Thread/interrupted)))))))

(deftest delegate-teardown-preserves-interruptions
  (doseq [[label error]
          [[:interrupted-exception
            (InterruptedException. "interrupted")]
           [:interrupted-io
            (java.io.InterruptedIOException. "interrupted")]
           [:closed-by-interrupt
            (java.nio.channels.ClosedByInterruptException.)]
           [:wrapped
            (ex-info "interrupted" {:kind :interrupted})]]]
    (testing (name label)
      (Thread/interrupted)
      (let [failure-state (harness/failure-state)
            events (atom [])
            delegate (failing-teardown-nemesis events error)
            subject (worker/wrap-nemesis-teardown
                     failure-state
                     :pause
                     (process/->PauseNemesis delegate (atom nil)))
            test {:nodes ["n1" "n2" "n3"]}]
        (try
          (let [[thrown interrupted?]
                (try
                  (nemesis/teardown! subject test)
                  [nil (.isInterrupted (Thread/currentThread))]
                  (catch Throwable throwable
                    [throwable (.isInterrupted (Thread/currentThread))]))]
            (is (identical? error thrown))
            (is interrupted?)
            (is (= [[:resume (:nodes test)] :teardown] @events))
            (is (nil? (harness/primary-failure failure-state))))
          (finally
            (Thread/interrupted)))))))

(deftest skips-disruptions-without-a-supported-leader
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        test {:nodes ["n1" "n2" "n3"]}
        operations [[(process/->ProcessNemesis delegate (atom nil))
                     {:type :info
                      :f :kill-process
                      :value :random}]
                    [(process/->PauseNemesis delegate (atom nil))
                     {:type :info
                      :f :pause-process
                      :value :random}]]]
    (with-redefs [cluster/membership-status (constantly nil)]
      (doseq [[subject op] operations]
        (is (= (skipped :no-supported-leader {})
               (:value (nemesis/invoke! subject test op))))))
    (is (empty? @invocations))))

(deftest skips-known-process-state-conflicts
  (let [invocations (atom [])
        delegate (recording-nemesis invocations)
        test {:nodes ["n1"]}
        subject (process/->ProcessNemesis delegate (atom nil))]
    (testing "there are no killed processes to restart"
      (is (= (skipped :no-processes-killed {})
             (:value (nemesis/invoke! subject
                                      test
                                      {:type :info
                                       :f :restart-process})))))

    (testing "a tracked disruption is already active"
      (let [active {:mode :random
                    :nodes ["n1"]}
            active-subject (process/->ProcessNemesis delegate
                                                     (atom active))
            value (:value
                   (nemesis/invoke! active-subject
                                    test
                                    {:type :info
                                     :f :kill-process
                                     :value :random}))]
        (is (= :skipped (:status value)))
        (is (= :processes-already-killed (:reason value)))
        (is (= active (:killed value)))))

    (is (empty? @invocations))))

(deftest requires-a-process-kill-and-an-intact-cluster
  (let [subject (#'process/coverage-checker)
        complete-history [{:f :kill-process
                           :value (installed {:mode :random
                                              :target-category :one})}
                          {:f :await-recovery
                           :value (installed {:leader "n1"})}]
        missing-mode-history [{:f :kill-process
                               :value (skipped
                                       :no-supported-leader
                                       {})}]
        unrecovered-history [{:f :kill-process
                              :value (installed {:mode :random})}
                             {:f :await-recovery
                              :value (indeterminate
                                      :recovery-timeout
                                      {})}]]
    (let [result (checker/check subject {} complete-history {})]
      (is (:valid? result))
      (is (= [:one] (:observed-target-categories result)))
      (is (= :intact (:cluster-state result))))
    (let [result (checker/check subject {} missing-mode-history {})]
      (is (false? (:valid? result)))
      (is (= [:random] (:missing-modes result)))
      (is (= :intact (:cluster-state result))))
    (let [result (checker/check subject {} unrecovered-history {})]
      (is (false? (:valid? result)))
      (is (empty? (:missing-modes result)))
      (is (= :degraded (:cluster-state result))))))

(defn- process-kill-value
  ([configs nodes]
   (process-kill-value configs (quorum/voter-set configs) nodes))
  ([configs reachable-voters nodes]
   (installed {:mode :random
               :nodes nodes
               :voter-configs configs
               :reachable-voters (vec reachable-voters)
               :stop-results (zipmap nodes (repeat :killed))})))

(defn- process-availability-history
  ([configs nodes client-events restart-time]
   (process-availability-history configs
                                 nodes
                                 client-events
                                 restart-time
                                 false))
  ([configs nodes client-events restart-time final-restart?]
   (into [{:time 0
           :type :info
           :process :nemesis
           :f :kill-process
           :value (process-kill-value configs nodes)}]
         (concat client-events
                 [(cond-> {:time restart-time
                           :type :info
                           :process :nemesis
                           :f :restart-process
                           :value nil}
                    final-restart?
                    (assoc :process-final-restart? true))]))))

(deftest standalone-process-requires-progress-when-voters-retain-quorum
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}]
        check (fn [nodes client-events restart-time]
                (checker/check
                 subject
                 {}
                 (process-availability-history configs
                                               nodes
                                               client-events
                                               restart-time)
                 {}))
        successful-attempt [{:time 5
                             :type :invoke
                             :process 0
                             :phase :main
                             :f :read}
                            {:time 10
                             :type :ok
                             :process 0
                             :phase :main
                             :f :read}]
        failed-attempt [{:time 5
                         :type :invoke
                         :process 0
                         :phase :main
                         :f :read}
                        {:time 10
                         :type :fail
                         :process 0
                         :phase :main
                         :f :read}]]
    (testing "a post-kill success proves retained-quorum availability"
      (let [result (check ["n1"] successful-attempt 40)]
        (is (:valid? result))
        (is (= 1 (:quorum-episode-count result)))
        (is (empty? (:failures result)))))

    (testing "client attempts without a success reject the run"
      (let [result (check ["n1"] failed-attempt 40)]
        (is (false? (:valid? result)))
        (is (= [:no-success-with-quorum]
               (mapv :reason (:failures result))))))

    (testing "missing client attempts are missing availability evidence"
      (let [result (check ["n1"] [] 40)]
        (is (false? (:valid? result)))
        (is (= [:no-client-attempts]
               (mapv :reason (:failures result))))))

    (testing "an early success proves availability before the window ends"
      (let [result (check ["n1"] successful-attempt 20)]
        (is (:valid? result))
        (is (true? (get-in result [:episodes 0 :evaluable?])))
        (is (empty? (:failures result)))))

    (testing "a short episode without success lacks enough evidence"
      (let [result (check ["n1"] failed-attempt 20)]
        (is (false? (:valid? result)))
        (is (= [:insufficient-observation-window]
               (mapv :reason (:failures result))))))

    (testing "a CAS version mismatch proves the service responded"
      (let [result (check ["n1"]
                          [{:time 5
                            :type :invoke
                            :process 0
                            :phase :main
                            :f :cas}
                           {:time 10
                            :type :fail
                            :process 0
                            :phase :main
                            :f :cas
                            :error :version-mismatch}]
                          40)]
        (is (:valid? result))
        (is (= 1 (get-in result [:episodes 0 :success-count])))))

    (testing "an operation invoked before the kill is not availability proof"
      (let [result (check ["n1"]
                          [{:time -5
                            :type :invoke
                            :process 0
                            :phase :main
                            :f :read}
                           {:time 5
                            :type :ok
                            :process 0
                            :phase :main
                            :f :read}]
                          40)]
        (is (false? (:valid? result)))
        (is (= [:no-client-attempts]
               (mapv :reason (:failures result))))))

    (testing "a success after the deadline does not satisfy availability"
      (let [result (check ["n1"]
                          [{:time 5
                            :type :invoke
                            :process 0
                            :phase :main
                            :f :read}
                           {:time 31
                            :type :ok
                            :process 0
                            :phase :main
                            :f :read}]
                          40)]
        (is (false? (:valid? result)))
        (is (= [:no-success-with-quorum]
               (mapv :reason (:failures result))))))

    (testing "a dangling invocation is not successful progress"
      (let [result (check ["n1"]
                          [{:time 5
                            :type :invoke
                            :process 0
                            :phase :main
                            :f :read}]
                          40)]
        (is (false? (:valid? result)))
        (is (= [:no-client-attempts]
               (mapv :reason (:failures result))))))))

(deftest excludes-completions-after-restart-from-process-availability
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}]
        history [{:time 0
                  :type :info
                  :process :nemesis
                  :f :kill-process
                  :value (process-kill-value configs ["n1"])}
                 {:time 5
                  :type :invoke
                  :process 0
                  :phase :main
                  :f :read}
                 {:time 30
                  :type :info
                  :process :nemesis
                  :f :restart-process
                  :value nil}
                 {:time 30
                  :type :ok
                  :process 0
                  :phase :main
                  :f :read}]
        result (checker/check subject {} history {})]
    (is (false? (:valid? result)))
    (is (= [:no-success-with-quorum]
           (mapv :reason (:failures result))))))

(deftest treats-a-short-final-process-episode-as-unevaluated
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}]
        history (process-availability-history configs
                                              ["n1"]
                                              []
                                              20
                                              true)
        result (checker/check subject {} history {})]
    (is (:valid? result))
    (is (= 1 (:truncated-episode-count result)))
    (is (zero? (:evaluated-quorum-episode-count result)))
    (is (empty? (:failures result)))))

(deftest standalone-process-does-not-require-progress-without-quorum
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}]
        history (process-availability-history configs
                                              ["n1" "n2"]
                                              []
                                              40)
        result (checker/check subject {} history {})]
    (is (:valid? result))
    (is (zero? (:quorum-episode-count result)))
    (is (= 1 (:no-quorum-episode-count result)))
    (is (empty? (:failures result)))))

(deftest standalone-process-rejects-write-success-without-quorum
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}]
        history (process-availability-history
                 configs
                 ["n1" "n2"]
                 [{:time 5
                   :type :invoke
                   :process 0
                   :phase :main
                   :f :write}
                  {:time 10
                   :type :ok
                   :process 0
                   :phase :main
                   :f :write}]
                 40)
        result (checker/check subject {} history {})]
    (is (false? (:valid? result)))
    (is (= [:unexpected-success-without-quorum]
           (mapv :reason (:failures result))))
    (is (= 1 (get-in result [:episodes 0 :unexpected-success-count])))))

(deftest standalone-process-uses-reachable-voters-for-quorum
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3" "n4" "n5"}]
        kill-value (process-kill-value configs
                                       ["n1" "n2" "n3"]
                                       ["n1"])
        history [{:time 0
                  :type :info
                  :process :nemesis
                  :f :kill-process
                  :value kill-value}
                 {:time 40
                  :type :info
                  :process :nemesis
                  :f :restart-process
                  :value nil}]
        result (checker/check subject {} history {})]
    (is (:valid? result))
    (is (false? (get-in result [:episodes 0 :quorum-retained?])))
    (is (= ["n2" "n3"] (get-in result [:episodes 0 :survivors])))))

(deftest standalone-process-allows-success-when-quorum-is-only-possible
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}]
        kill-value (process-kill-value configs
                                       ["n1" "n2"]
                                       ["n1"])
        history [{:time 0
                  :type :info
                  :process :nemesis
                  :f :kill-process
                  :value kill-value}
                 {:time 5
                  :type :invoke
                  :process 0
                  :phase :main
                  :f :write}
                 {:time 10
                  :type :ok
                  :process 0
                  :phase :main
                  :f :write}
                 {:time 40
                  :type :info
                  :process :nemesis
                  :f :restart-process
                  :value nil}]
        result (checker/check subject {} history {})]
    (is (:valid? result))
    (is (false? (get-in result [:episodes 0 :quorum-retained?])))
    (is (true? (get-in result
                       [:episodes 0 :configured-quorum-retained?])))
    (is (= 1 (:indeterminate-quorum-episode-count result)))
    (is (zero? (get-in result
                       [:episodes 0 :unexpected-success-count])))
    (is (empty? (:failures result)))))

(deftest standalone-process-excludes-learners-from-quorum
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}]
        history (process-availability-history configs ["n4"] [] 40)
        result (checker/check subject {} history {})]
    (is (false? (:valid? result)))
    (is (= 1 (:quorum-episode-count result)))
    (is (= [:no-client-attempts]
           (mapv :reason (:failures result))))))

(deftest standalone-process-checks-every-joint-voter-config
  (let [subject (#'process/availability-checker 30)
        configs [#{"n1" "n2" "n3"}
                 #{"n3" "n4" "n5"}]
        no-quorum-history (process-availability-history configs
                                                        ["n1" "n2"]
                                                        []
                                                        40)
        retained-history (process-availability-history configs
                                                       ["n1"]
                                                       []
                                                       40)]
    (is (:valid? (checker/check subject {} no-quorum-history {})))
    (is (= [:no-client-attempts]
           (->> (checker/check subject {} retained-history {})
                :failures
                (mapv :reason))))))

(deftest chaos-reuses-random-kills-without-standalone-availability-verdicts
  (let [package (process/process-package nil)
        raw-checker (:checker package)
        chaos-checker (#'openraft-nemesis/fault-class-checker raw-checker)
        configs [#{"n1" "n2" "n3"}]
        history (into (process-availability-history configs
                                                    ["n1"]
                                                    []
                                                    40)
                      [{:time 41
                        :type :info
                        :process :nemesis
                        :f :await-recovery
                        :value (installed {:leader "n2"})}])
        standalone-result (checker/check raw-checker {} history {})
        chaos-result (checker/check chaos-checker {} history {})]
    (is (false? (:valid? standalone-result)))
    (is (= [:random] (:observed-modes standalone-result)))
    (is (= :intact (:cluster-state standalone-result)))
    (is (false? (get-in standalone-result [:availability :valid?])))
    (is (:valid? chaos-result))
    (is (:fault-class-executed? chaos-result))
    (is (false? (get-in chaos-result [:availability :valid?])))))

(deftest reports-an-indeterminate-process-state
  (let [subject (#'process/coverage-checker)
        covered-history [{:f :kill-process
                          :value (installed {:mode :random})}
                         {:f :await-recovery
                          :value (installed {:leader "n1"})}]
        indeterminate-history (conj covered-history
                                    {:f :kill-process
                                     :value (indeterminate
                                             :effect-unknown
                                             {:mode :random})})
        recovered-history (conj indeterminate-history
                                {:f :await-recovery
                                 :value (installed {:leader "n2"})})]
    (let [result (checker/check subject {} indeterminate-history {})]
      (is (= :unknown (:valid? result)))
      (is (= :unknown (:cluster-state result))))
    (let [result (checker/check subject {} recovered-history {})]
      (is (= :unknown (:valid? result)))
      (is (= :unknown (:cluster-state result))))))

(defn- pause-availability-history
  [configs nodes client-events resume-time]
  (into [{:time 0
          :type :info
          :process :nemesis
          :f :pause-process
          :value (installed
                  {:mode :random
                   :nodes nodes
                   :voter-configs configs
                   :reachable-voters (vec (quorum/voter-set configs))
                   :pause-results (zipmap nodes (repeat :paused))})}]
        (concat client-events
                [{:time resume-time
                  :type :info
                  :process :nemesis
                  :f :resume-process
                  :value nil}])))

(deftest standalone-pause-applies-quorum-aware-availability
  (let [subject (#'process/availability-checker
                 30
                 :pause-process
                 :resume-process
                 process/required-pause-modes
                 :pause-final-resume?)
        configs [#{"n1" "n2" "n3"}]
        success [{:time 5
                  :type :invoke
                  :process 0
                  :phase :main
                  :f :read}
                 {:time 10
                  :type :ok
                  :process 0
                  :phase :main
                  :f :read}]
        retained (checker/check
                  subject
                  {}
                  (pause-availability-history configs ["n1"] success 40)
                  {})
        retained-without-progress (checker/check
                                   subject
                                   {}
                                   (pause-availability-history configs
                                                               ["n1"]
                                                               []
                                                               40)
                                   {})
        no-quorum (checker/check
                   subject
                   {}
                   (pause-availability-history configs
                                               ["n1" "n2"]
                                               []
                                               40)
                   {})]
    (is (:valid? retained))
    (is (false? (:valid? retained-without-progress)))
    (is (= :no-client-attempts
           (get-in retained-without-progress [:failures 0 :reason])))
    (is (:valid? no-quorum))
    (is (= 1 (:no-quorum-episode-count no-quorum)))))

(deftest standalone-pause-accepts-progress-throughout-the-pause-episode
  (let [second 1000000000
        subject (#'process/availability-checker
                 nil
                 :pause-process
                 :resume-process
                 process/required-pause-modes
                 :pause-final-resume?)
        configs [#{"n1" "n2" "n3"}]
        history (pause-availability-history
                 configs
                 ["n1"]
                 [{:time (* 50 1000000)
                   :type :invoke
                   :process 0
                   :phase :main
                   :f :read}
                  {:time (+ (* 5 second) (* 20 1000000))
                   :type :ok
                   :process 0
                   :phase :main
                   :f :read}]
                 (* 10 second))
        result (checker/check subject {} history {})]
    (is (:valid? result))
    (is (= 1 (get-in result [:episodes 0 :success-count])))
    (is (empty? (:failures result)))))

(deftest checks-pause-coverage-and-recovery-state
  (let [subject (#'process/pause-coverage-checker)
        test {:nodes voters}
        check #(checker/check subject test % {})
        pause-value (fn [mode nodes]
                      (installed
                       {:mode mode
                        :leader "n1"
                        :nodes nodes
                        :voter-configs [(set voters)]
                        :target-category (outcome/target-category
                                          (count voters)
                                          (count nodes))
                        :pause-results (zipmap nodes (repeat :paused))}))
        follower-targets (pause-value :random ["n2" "n3"])
        leader-targets (pause-value :random ["n1" "n2"])
        resumed-all (installed
                     {:paused nil
                      :resumed voters
                      :resume-results (zipmap voters (repeat :resumed))})
        complete-history [{:f :pause-process
                           :value follower-targets}
                          {:f :resume-process
                           :value resumed-all}
                          {:f :await-recovery
                           :value (installed {:leader "n1"})}
                          {:f :pause-process
                           :value leader-targets}
                          {:f :resume-process
                           :value resumed-all}
                          {:f :await-recovery
                           :value (installed {:leader "n2"})}]
        missing-mode-history [{:f :pause-process
                               :value (skipped
                                       :no-supported-leader
                                       {})}]
        unrecovered-history (-> complete-history
                                pop
                                (conj {:f :await-recovery
                                       :value (indeterminate
                                               :recovery-timeout
                                               {})}))
        indeterminate-history (conj complete-history
                                    {:f :pause-process
                                     :value (indeterminate
                                             :effect-unknown
                                             {:mode :random})})
        paused-history (conj complete-history
                             {:f :pause-process
                              :value leader-targets})
        resuming-history (conj paused-history
                               {:type :info
                                :f :resume-process})
        recovered-history (into resuming-history
                                [{:f :resume-process
                                  :value resumed-all}
                                 {:f :await-recovery
                                  :value (installed {:leader "n2"})}])
        partial-resume-history (assoc-in complete-history
                                         [4 :value :resumed]
                                         ["n1"])
        mixed-pause (assoc-in leader-targets
                              [:pause-results "n1"]
                              :target-absent)
        mixed-pause-history (conj (subvec complete-history 0 3)
                                  {:f :pause-process
                                   :value mixed-pause})
        covered-then-mixed-pause-history
        (conj complete-history
              {:f :pause-process
               :value mixed-pause})
        mixed-resume-history (assoc-in complete-history
                                       [4 :value :resume-results "n5"]
                                       :target-absent)
        mixed-resume-history (assoc-in mixed-resume-history
                                       [4 :value :resume-results "n4"]
                                       :target-already-exited)
        missing-resume-result-history
        (update-in complete-history
                   [4 :value :resume-results]
                   dissoc
                   "n5")
        no-resume-history
        (assoc-in complete-history
                  [4 :value :resume-results]
                  (zipmap voters (repeat :target-absent)))]
    (testing "random pause episodes complete and recover"
      (is (= {:valid? true
              :observed-target-categories [:minority]
              :cluster-state :intact}
             (select-keys (check complete-history)
                          [:valid?
                           :observed-target-categories
                           :cluster-state]))))

    (testing "the random pause mode is missing"
      (is (= {:valid? false
              :missing-modes [:random]
              :cluster-state :intact}
             (select-keys (check missing-mode-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "the final resume is not followed by recovery"
      (is (= {:valid? false
              :missing-modes []
              :cluster-state :recovery-pending}
             (select-keys (check unrecovered-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "a pause result is indeterminate"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check indeterminate-history)
                          [:valid? :cluster-state]))))

    (testing "a resume invocation makes a paused state indeterminate"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check resuming-history)
                          [:valid? :cluster-state]))))

    (testing "a partial resume remains indeterminate after recovery"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check partial-resume-history)
                          [:valid? :cluster-state]))))

    (testing "mixed pause evidence still confirms a random disruption"
      (is (= {:valid? false
              :missing-modes []
              :cluster-state :paused}
             (select-keys (check mixed-pause-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "a terminal mixed pause invalidates prior complete coverage"
      (is (= {:valid? false
              :missing-modes []
              :cluster-state :paused}
             (select-keys (check covered-then-mixed-pause-history)
                          [:valid? :missing-modes :cluster-state]))))

    (testing "mixed evidence confirms every target has no remaining pause"
      (is (= {:valid? true
              :cluster-state :intact}
             (select-keys (check mixed-resume-history)
                          [:valid? :cluster-state]))))

    (testing "missing resume evidence does not confirm recovery"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check missing-resume-result-history)
                          [:valid? :cluster-state]))))

    (testing "an installed resume requires an actual resumed process"
      (is (= {:valid? :unknown
              :cluster-state :unknown}
             (select-keys (check no-resume-history)
                          [:valid? :cluster-state]))))

    (testing "resume completion and recovery resolve an in-flight resume"
      (is (= {:valid? true
              :cluster-state :intact}
             (select-keys (check recovered-history)
                          [:valid? :cluster-state]))))))
