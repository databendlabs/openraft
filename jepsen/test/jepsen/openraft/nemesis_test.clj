(ns jepsen.openraft.nemesis-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.nemesis :as nemesis]
            [jepsen.openraft [await :as await]
             [cluster :as cluster]
             [db :as db]
             [harness :as harness]
             [nemesis :as openraft-nemesis]
             [worker :as worker]]
            [jepsen.openraft.nemesis.membership :as membership]
            [jepsen.openraft.nemesis.partition :as partition]
            [jepsen.openraft.nemesis.process :as process]))

(def test-config
  {:nodes ["n1" "n2" "n3" "n4" "n5"]})

(deftest schedules-and-retries-skipped-faults
  (doseq [skip-result (concat
                       (map (fn [reason]
                              {:status :skipped
                               :reason reason})
                            [:no-quorum-safe-process-target
                             :no-reachable-pause-target
                             :no-safe-partition-target
                             :no-supported-leader])
                       [{:status :no-supported-leader}
                        :no-supported-leader])]
    (testing (str skip-result)
      (let [faults (#'openraft-nemesis/interval-schedule
                    10
                    3
                    (gen/cycle [{:type :info :f :first}
                                {:type :info :f :second}]))
            generator (gen/shortest-any
                       (gen/nemesis (gen/time-limit 40 faults))
                       (gen/clients
                        (gen/stagger 0.1 (repeat {:f :read}))))
            completion-latency (gen/secs->nanos 2)
            first-attempt? (atom true)
            history (gen-test/simulate
                     generator
                     (fn [_ operation]
                       (let [skip? (and (= :first (:f operation))
                                        (compare-and-set! first-attempt?
                                                          true
                                                          false))]
                         (cond-> (assoc operation :type :ok)
                           skip? (assoc :value skip-result)
                           skip? (update :time + completion-latency)))))
            fault-ops (->> history
                           (filter #(and (= :nemesis (:process %))
                                         (= :info (:type %))))
                           (take 3))
            [_ retry next-fault] fault-ops]
        (is (= [:first :first :second] (mapv :f fault-ops)))
        (is (= (gen/secs->nanos 3) (:time (first fault-ops))))
        (is (<= (gen/secs->nanos 5.5)
                (:time retry)
                (gen/secs->nanos 6.5)))
        (is (<= (+ (:time retry) (gen/secs->nanos 5))
                (:time next-fault)
                (+ (:time retry) (gen/secs->nanos 15))))))))

(deftest cleans-up-all-faults-before-confirming-recovery
  (let [database (db/db {})
        package (openraft-nemesis/compose-packages
                 [(membership/membership-package
                   database
                   test-config)
                  (partition/partition-package)
                  (process/process-package database)
                  (process/pause-package database)])]
    (is (= [:stop-partition
            :restart-process
            :resume-process
            :restore-membership
            :await-recovery]
           (mapv :f (:final-generator package))))))

(deftest requires-each-composed-fault-class-to-execute
  (let [database (db/db {})
        all-voters (set (:nodes test-config))
        fewer-voters (disj all-voters "n5")
        package (openraft-nemesis/compose-packages
                 [(partition/partition-package)
                  (process/pause-package database)
                  (membership/membership-package database test-config)])
        check (fn [history]
                (checker/check (:checker package)
                               test-config
                               history
                               {}))
        history [{:f :start-partition
                  :value {:status :installed
                          :mode :leader-in-majority}}
                 {:f :stop-partition
                  :value {:status :installed}}
                 {:f :pause-process
                  :value {:status :installed
                          :mode :leader-unpaused
                          :leader "n1"
                          :nodes ["n2"]
                          :voter-configs [all-voters]
                          :survivors (vec (disj all-voters "n2"))
                          :pause-results {"n2" :paused}}}
                 {:f :resume-process
                  :value {:status :installed
                          :paused nil
                          :resumed (:nodes test-config)
                          :resume-results
                          (zipmap (:nodes test-config) (repeat :resumed))}}
                 {:f :shrink
                  :value {:status :installed
                          :change :shrink
                          :node "n5"
                          :leader "n1"
                          :before all-voters
                          :after fewer-voters}}
                 {:f :restore-membership
                  :value {:status :installed
                          :leader "n1"
                          :voters all-voters}}
                 {:f :await-recovery
                  :value {:status :installed
                          :leader "n1"}}]]
    (testing "one successful mode covers each fault class in chaos"
      (let [result (check history)]
        (is (:valid? result))
        (is (true? (get-in result [:partition :fault-class-executed?])))
        (is (true? (get-in result [:pause :fault-class-executed?])))
        (is (true? (get-in result
                           [:membership :fault-class-executed?])))))

    (testing "a skipped fault does not count as execution"
      (let [result (check (mapv #(if (= :pause-process (:f %))
                                   (assoc % :value
                                          {:status :skipped
                                           :reason :no-supported-leader})
                                   %)
                                history))]
        (is (false? (:valid? result)))
        (is (false? (get-in result [:pause :fault-class-executed?])))))))

(defn- condition-timeout [condition]
  (try
    (await/until! condition
                  #(await/retry! condition {})
                  {:timeout 0
                   :retry-interval 0})
    nil
    (catch Exception e
      e)))

(deftest classifies-recovery-outcomes
  (let [op {:type :info
            :process :nemesis
            :f :await-recovery}
        subject (openraft-nemesis/->RecoveryNemesis)]
    (testing "confirmed recovery is installed"
      (with-redefs [cluster/await-ready! (constantly {:leader "n2"})]
        (is (= {:status :installed
                :leader "n2"}
               (:value (nemesis/invoke! subject test-config op))))))

    (testing "a modeled readiness timeout is indeterminate"
      (let [error (condition-timeout :cluster-ready)]
        (with-redefs [cluster/await-ready! (fn [_test] (throw error))]
          (let [value (:value (nemesis/invoke! subject test-config op))]
            (is (= :indeterminate (:status value)))
            (is (= :recovery-timeout (:reason value)))))))

    (testing "an unexpected recovery exception takes the Harness path"
      (let [failure-state (harness/failure-state)
            error (RuntimeException. "recovery bug")
            wrapped (worker/wrap-nemesis failure-state subject)
            setup-subject (nemesis/setup! wrapped test-config)
            thrown (with-redefs [cluster/await-ready! (fn [_test]
                                                        (throw error))]
                     (try
                       (nemesis/invoke! setup-subject test-config op)
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))
        (is (identical? error
                        (:throwable
                         (harness/primary-failure failure-state))))))

    (testing "an unrelated timeout tag takes the Harness path"
      (let [failure-state (harness/failure-state)
            error (ex-info "unknown timeout" {:type :timeout})
            wrapped (worker/wrap-nemesis failure-state subject)
            setup-subject (nemesis/setup! wrapped test-config)
            thrown (with-redefs [cluster/await-ready! (fn [_test]
                                                        (throw error))]
                     (try
                       (nemesis/invoke! setup-subject test-config op)
                       nil
                       (catch Exception e
                         e)))]
        (is (identical? error thrown))
        (is (identical? error
                        (:throwable
                         (harness/primary-failure failure-state))))))

    (testing "interruption takes priority over timeout classification"
      (doseq [[label make-error]
              [[:interrupted #(InterruptedException. "interrupted")]
               [:interrupted-io
                #(java.io.InterruptedIOException. "interrupted")]
               [:closed-by-interrupt
                #(java.nio.channels.ClosedByInterruptException.)]
               [:wrapped
                #(ex-info "interrupted"
                          {:kind :interrupted
                           :type :timeout})]]]
        (Thread/interrupted)
        (try
          (let [failure-state (harness/failure-state)
                error (make-error)
                wrapped (worker/wrap-nemesis failure-state subject)
                setup-subject (nemesis/setup! wrapped test-config)
                [thrown interrupted?]
                (with-redefs [cluster/await-ready!
                              (fn [_test] (throw error))]
                  (try
                    (nemesis/invoke! setup-subject test-config op)
                    [nil (.isInterrupted (Thread/currentThread))]
                    (catch Exception e
                      [e (.isInterrupted (Thread/currentThread))])))]
            (is (identical? error thrown) (name label))
            (is interrupted? (name label))
            (is (nil? (harness/primary-failure failure-state)) (name label)))
          (finally
            (Thread/interrupted)))))))
